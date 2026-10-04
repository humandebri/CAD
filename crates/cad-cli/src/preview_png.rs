//! In-process raster previews; no browser, shell or system font dependency.
use miette::{IntoDiagnostic, Result, miette};
use resvg::{tiny_skia, usvg};

pub fn render(svg: &str, dpi: f64) -> Result<Vec<u8>> {
    if !dpi.is_finite() || !(36.0..=300.0).contains(&dpi) {
        return Err(miette!("preview DPI must be 36..300"));
    }
    let mut options = usvg::Options {
        dpi: dpi as f32,
        ..Default::default()
    };
    options.fontdb_mut().load_font_data(
        include_bytes!("../../cad-render-pdf/assets/mplus/mplus-1p-regular.ttf").to_vec(),
    );
    let family = options
        .fontdb
        .faces()
        .next()
        .and_then(|f| f.families.first())
        .map(|f| f.0.clone())
        .ok_or_else(|| miette!("bundled preview font could not be loaded"))?;
    options.font_family = family.clone();
    options.fontdb_mut().set_sans_serif_family(&family);
    options.fontdb_mut().set_serif_family(&family);
    options.fontdb_mut().set_monospace_family(&family);
    // CAD SVGs are responsive (100%); their model-space viewBox is not a
    // pixel size. Supply physical paper dimensions only for rasterization.
    let raster_svg = physical_size(svg)?;
    let tree = usvg::Tree::from_str(&raster_svg, &options).into_diagnostic()?;
    let size = tree.size().to_int_size();
    if u64::from(size.width()) * u64::from(size.height()) > 40_000_000 {
        return Err(miette!("preview exceeds 40 million pixels; reduce DPI"));
    }
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or_else(|| miette!("could not allocate preview"))?;
    pixmap.fill(tiny_skia::Color::WHITE);
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().into_diagnostic()
}

fn physical_size(svg: &str) -> Result<String> {
    let root = svg.split('>').next().unwrap_or(svg);
    let attribute = |name: &str| -> Option<f64> {
        root.split_once(&format!("{name}=\""))?
            .1
            .split('"')
            .next()?
            .parse()
            .ok()
    };
    match (
        attribute("data-paper-width-mm"),
        attribute("data-paper-height-mm"),
    ) {
        (Some(w), Some(h)) if w.is_finite() && h.is_finite() && w > 0. && h > 0. => Ok(svg
            .replacen("width=\"100%\"", &format!("width=\"{w}mm\""), 1)
            .replacen("height=\"100%\"", &format!("height=\"{h}mm\""), 1)),
        (None, None) => Ok(svg.to_owned()),
        _ => Err(miette!("invalid SVG paper dimensions")),
    }
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn gallery(pages: &[(String, String)], dpi: f64) -> String {
    let mut html = format!(
        "<!doctype html><html lang=\"ja\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>CAD 図面確認</title><style>body{{font:16px sans-serif;max-width:1100px;margin:32px auto;padding:0 20px;color:#22303b;background:#f4f5f7}}img{{width:100%;height:auto;border:1px solid #cbd1d8;background:white}}article{{margin:32px 0}}a{{color:#126b79}}</style><h1>CAD 図面確認</h1><p>静的検査: 合格 / 見た目の確認: 未実施</p><p><a href=\"set.pdf\">PDF</a> · <a href=\"manifest.json\">検査と元データの版</a></p><p>PNGはSVGから生成（{dpi} dpi）。フォントは同梱Mplusへ代替。SVGは用紙外の図形も表示する場合があるため、PDFの収まり・フォントは別途確認してください。</p>"
    );
    for (i, (drawing, layout)) in pages.iter().enumerate() {
        let prefix = format!("{:03}", i + 1);
        html.push_str(&format!("<article><h2>{} / {}</h2><p><a href=\"{prefix}.svg\">SVG</a> · <a href=\"{prefix}.png\">PNG</a></p><img src=\"{prefix}.png\" alt=\"{}\" loading=\"lazy\"></article>",escape(drawing),escape(layout),escape(drawing)));
    }
    html.push_str("</html>");
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn japanese_annotations_render_with_bundled_font_when_requested_font_is_absent() {
        let empty = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="20mm" viewBox="0 0 400 80"></svg>"#;
        let text = empty.replace("</svg>",r#"<text x="10" y="45" font-size="24" font-family="Nonexistent Drafting Font">天井高2400mm</text></svg>"#);
        assert_ne!(
            render(empty, 72.).unwrap(),
            render(&text, 72.).unwrap(),
            "text must not silently disappear"
        );
    }
    #[test]
    fn physical_page_dimensions_follow_dpi_and_invalid_allocations_are_rejected() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="25.4mm" height="12.7mm" viewBox="0 0 100 50"><path d="M0 0L100 50" stroke="black"/></svg>"#;
        let png = render(svg, 144.).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 144);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 72);
        let responsive = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100%" height="100%" viewBox="0 0 21000 14850" data-paper-width-mm="420" data-paper-height-mm="297"/>"#;
        let png = render(responsive, 144.).unwrap();
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 2381);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 1684);
        assert!(render(svg, f64::NAN).is_err());
        assert!(
            render(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="100000mm" height="100000mm"/>"#,
                300.
            )
            .is_err()
        );
        let html = gallery(&[("<script>".into(), "\"default\"".into())], 144.);
        assert!(!html.contains("<script>"));
    }
}
