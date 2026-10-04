let clipNamespace=0;
export function sanitizeSvg(svgText: string): string {
  const document = new DOMParser().parseFromString(svgText, "image/svg+xml");
  if (document.querySelector("parsererror") !== null) {
    throw new Error("SVG parse failed");
  }
  document.querySelectorAll("script, foreignObject, image, use, iframe, object, embed").forEach((element) => element.remove());
  const clips=new Map<string,string>();
  const prefix=`cad-clip-${++clipNamespace}-`;
  document.querySelectorAll("clipPath[id]").forEach((element,index)=>{
    const id=element.getAttribute("id")!;
    if(/^[A-Za-z0-9_-]+$/.test(id)) {const replacement=`${prefix}${index}`;clips.set(id,replacement);element.setAttribute("id",replacement);}
  });
  document.querySelectorAll("*").forEach((element) => {
    Array.from(element.attributes).forEach((attribute) => {
      const attributeName = attribute.name.toLowerCase();
      const attributeValue = attribute.value.trim().toLowerCase();
      const localClip=attributeName==="clip-path" ? /^url\(#([A-Za-z0-9_-]+)\)$/.exec(attribute.value.trim()) : null;
      if(localClip!==null && clips.has(localClip[1])) {element.setAttribute(attribute.name,`url(#${clips.get(localClip[1])})`);return;}
      if (
        attributeName.startsWith("on")
        || attributeName === "href"
        || attributeName.endsWith(":href")
        || attributeValue.includes("javascript:")
        || attributeValue.includes("url(")
      ) {
        element.removeAttribute(attribute.name);
      }
    });
  });
  return new XMLSerializer().serializeToString(document.documentElement);
}
