import { expect, test } from "@playwright/test";

test("massing tools preserve explicit conditions, invalidate changed previews and save the last report", async ({ page }) => {
  await page.goto("/tests/fixtures/toolkit.html?massing");
  const tool=page.getByRole("combobox",{name:"Building tool",exact:true});
  const preview=page.getByRole("button",{name:"Preview building tool"});
  const apply=page.getByRole("button",{name:"Apply building preview"});
  await tool.selectOption("massing_projection");
  await page.getByRole("textbox",{name:"Projection origin east,north",exact:true}).fill("0,0");
  await page.getByRole("textbox",{name:"Diagram origin x,y",exact:true}).fill("10000,20000");
  await page.getByRole("spinbutton",{name:"Building height mm",exact:true}).fill("5000");
  await page.getByRole("spinbutton",{name:"Projection yaw degrees",exact:true}).fill("30");
  await page.getByRole("spinbutton",{name:"View elevation degrees",exact:true}).fill("45");
  await preview.click(); await apply.click();
  expect(JSON.parse(await page.locator("output").innerText()).geometry).toEqual({type:"massing_projection",footprint_ids:["footprint"],height_mm:5000,origin:[0,0],at:[10000,20000],yaw_deg:30,elevation_deg:45});
  await tool.selectOption("sun_shadow");
  await page.getByRole("spinbutton",{name:"Solar azimuth degrees",exact:true}).fill("90");
  await page.getByRole("spinbutton",{name:"Solar altitude degrees",exact:true}).fill("45");
  await preview.click();
  await page.getByRole("spinbutton",{name:"Solar altitude degrees",exact:true}).fill("50");
  await expect(apply).toBeDisabled();
  await preview.click(); await apply.click();
  expect(JSON.parse(await page.locator("output").innerText()).geometry).toEqual({type:"sun_shadow",footprint_ids:["footprint"],height_mm:5000,sun_azimuth_deg:90,sun_altitude_deg:50});
  await tool.selectOption("sky_view");
  await page.getByRole("textbox",{name:"Observer east,north",exact:true}).fill("-5000,-5000");
  await page.getByRole("spinbutton",{name:"Observer height mm",exact:true}).fill("1000");
  await page.getByRole("spinbutton",{name:"Sky azimuth samples",exact:true}).fill("35");
  await preview.click(); await expect(apply).toBeDisabled();
  await expect(page.locator(".toolkit-panel > p[role=status]")).toContainText("36 to 2048");
  await page.getByRole("spinbutton",{name:"Sky azimuth samples",exact:true}).fill("180");
  await page.getByRole("spinbutton",{name:"Sky diagram radius mm",exact:true}).fill("1000");
  await preview.click(); await apply.click();
  expect(JSON.parse(await page.locator("output").innerText()).geometry).toEqual({type:"sky_view",footprint_ids:["footprint"],height_mm:5000,observer:[-5000,-5000,1000],azimuth_samples:180,at:[10000,20000],radius_mm:1000});
  const report=page.getByRole("textbox",{name:"Massing calculation report JSON",exact:true});
  const frozen=await report.inputValue();
  await page.getByRole("button",{name:"Save new massing report"}).click();
  await expect(page.locator(".massing-report p[role=status]")).toContainText("cancelled");
  await tool.selectOption("door"); await expect(report).toHaveValue(frozen);
  await report.focus(); await page.keyboard.press("Enter"); await page.keyboard.press("Tab");
  await expect(page.getByRole("textbox",{name:"Massing report file",exact:true})).toBeFocused();
  await page.keyboard.type("/reports/sky.json"); await page.keyboard.press("Tab"); await page.keyboard.press("Enter");
  expect(JSON.parse(await page.locator("output").innerText())).toEqual({path:"/reports/sky.json",report:JSON.parse(frozen)});
  await page.setViewportSize({width:390,height:844});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth)).toBe(true);
});

test("massing without selected closed footprints cannot be applied",async({page})=>{
  await page.goto("/tests/fixtures/toolkit.html");
  await page.getByRole("combobox",{name:"Building tool",exact:true}).selectOption("sun_shadow");
  await page.getByRole("button",{name:"Preview building tool"}).click();
  await expect(page.locator(".toolkit-panel > p[role=status]")).toContainText("Select closed");
  await expect(page.getByRole("button",{name:"Apply building preview"})).toBeDisabled();
});

test("calculated text passes the expression and explicit formatting without evaluating it in the browser", async ({ page }) => {
  await page.goto("/tests/fixtures/toolkit.html");
  await page.getByRole("combobox", {name:"Building tool",exact:true}).selectOption("calculated_text");
  await page.getByRole("textbox", {name:"Origin x,y",exact:true}).fill("100,200");
  await page.getByRole("textbox", {name:"Arithmetic expression",exact:true}).fill("1000*2000/1e6");
  await page.getByRole("textbox", {name:"Text prefix",exact:true}).fill("面積: ");
  await page.getByRole("textbox", {name:"Text suffix",exact:true}).fill(" m²");
  await page.getByRole("spinbutton", {name:"Decimal places",exact:true}).fill("13");
  await page.getByRole("button", {name:"Preview building tool"}).click();
  await expect(page.locator(".toolkit-panel p[role=status]")).toContainText("0 to 12");
  await expect(page.getByRole("button", {name:"Apply building preview"})).toBeDisabled();
  await page.getByRole("spinbutton", {name:"Decimal places",exact:true}).fill("2");
  await page.getByRole("button", {name:"Preview building tool"}).click();
  await expect(page.locator("output")).toHaveText("Choose a tool.");
  await page.getByRole("button", {name:"Apply building preview"}).click();
  expect(JSON.parse(await page.locator("output").innerText()).geometry).toEqual({type:"calculated_text",at:[100,200],expression:"1000*2000/1e6",precision:2,prefix:"面積: ",suffix:" m²",style:"note",rotation_deg:0});
});

test("tangent circle previews a radius and hint against two selected lines", async ({ page }) => {
  await page.goto("/tests/fixtures/toolkit.html?tangent");
  await page.getByRole("combobox", { name: "Building tool", exact: true }).selectOption("tangent_circle");
  await page.getByRole("textbox", { name: "Near center x,y", exact: true }).fill("10,10");
  await page.getByRole("spinbutton", { name: "Radius", exact: true }).fill("5");
  await expect(page.locator(".toolkit-panel")).toContainText("infinite extensions");
  await page.getByRole("button", { name: "Preview building tool" }).click();
  await expect(page.locator("output")).toHaveText("Choose a tool.");
  await page.getByRole("spinbutton", { name: "Radius", exact: true }).fill("6");
  await expect(page.getByRole("button", { name: "Apply building preview" })).toBeDisabled();
  await page.getByRole("spinbutton", { name: "Radius", exact: true }).fill("5");
  await page.getByRole("button", { name: "Preview building tool" }).click();
  await page.getByRole("button", { name: "Apply building preview" }).click();
  expect(JSON.parse(await page.locator("output").innerText()).geometry).toEqual({
    type: "tangent_circle", line_ids: ["horizontal-line", "vertical-line"], radius: 5, near: [10,10],
  });
  await page.goto("/tests/fixtures/toolkit.html");
  await page.getByRole("combobox", { name: "Building tool", exact: true }).selectOption("tangent_circle");
  await page.getByRole("button", { name: "Preview building tool" }).click();
  await expect(page.locator("p[role=status]")).toContainText("Select two distinct straight lines");
  await expect(page.getByRole("button", { name: "Apply building preview" })).toBeDisabled();
});

test("building tools stage geometry before application and report measurement exclusions", async ({ page }) => {
  await page.goto("/tests/fixtures/toolkit.html");
  const apply = page.getByRole("button", { name: "Apply building preview" });
  await expect(apply).toBeDisabled();
  await page.getByRole("combobox", { name: "Building tool", exact: true }).selectOption("door");
  await page.getByRole("textbox", { name: "Origin x,y", exact: true }).fill("100,200");
  await page.getByRole("spinbutton", { name: "Width", exact: true }).fill("900");
  await page.getByRole("spinbutton", { name: "Swing degrees", exact: true }).fill("-90");
  await page.getByRole("button", { name: "Preview building tool" }).click();
  await expect(page.locator("output")).toHaveText("Choose a tool.");
  await expect(apply).toBeEnabled();
  await apply.click();
  const result = JSON.parse(await page.locator("output").innerText());
  expect(result.geometry).toEqual({ type: "door", hinge: [100, 200], width: 900, rotation_deg: 0, swing_deg: -90 });
  await page.getByRole("button", { name: "Measure drawing" }).click();
  await expect(page.getByLabel("Measured geometry")).toContainText("12.0000 m");
  await expect(page.getByLabel("Measured geometry")).toContainText("2.0000 m²");
  await expect(page.getByLabel("Measured geometry")).toContainText("Unsupported entities excluded: 1");
});

test("building tool invalid coordinates do not enable application", async ({ page }) => {
  await page.goto("/tests/fixtures/toolkit.html");
  await page.getByRole("textbox", { name: "First point x,y" }).fill("wrong");
  await page.getByRole("button", { name: "Preview building tool" }).click();
  await expect(page.locator("p[role=status]")).toContainText("Coordinates must be x,y");
  await expect(page.getByRole("button", { name: "Apply building preview" })).toBeDisabled();
});


test("union measurement excludes overlap and rejects invalid approximation tolerance", async ({ page }) => {
  await page.goto("/tests/fixtures/toolkit.html");
  await page.getByRole("button", { name: "Measure drawing" }).click();
  await expect(page.getByLabel("Measured geometry")).toContainText("2.0000 m²");
  await page.getByRole("combobox", { name: "Area calculation" }).selectOption("union");
  await expect(page.getByLabel("Measured geometry")).toHaveCount(0);
  await page.getByRole("spinbutton", { name: "Area curve tolerance mm" }).fill("-1");
  await page.getByRole("button", { name: "Measure drawing" }).click();
  await expect(page.locator("p[role=status]")).toContainText("must be positive");
  await expect(page.getByLabel("Measured geometry")).toHaveCount(0);
  await page.getByRole("spinbutton", { name: "Area curve tolerance mm" }).fill("0.01");
  await page.getByRole("button", { name: "Measure drawing" }).click();
  await expect(page.getByLabel("Measured geometry")).toContainText("1.5000 m²");
  await expect(page.getByLabel("Measured geometry")).toContainText("Sum before overlap removal: 2.0000 m²");
  await expect(page.getByLabel("Measured geometry")).toContainText("Curve tolerance: 0.01 mm");
  await expect(page.getByLabel("Measured geometry")).toContainText("error estimate");
});
