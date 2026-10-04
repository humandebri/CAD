import { expect, test } from "@playwright/test";
import { finiteDraftNumber, repeatableCommand, initialDraftingOptions, retuneDraftOperation } from "../../src/drafting-panel";


test("drafting numeric input rejects empty and non-finite values", () => {
  for (const value of [" ", "Infinity"]) expect(finiteDraftNumber(value)).toBeNull();
  expect(finiteDraftNumber("-300")).toBe(-300);
  expect(finiteDraftNumber("0")).toBe(0);
  expect(finiteDraftNumber("1.25")).toBe(1.25);
});

test("repeat only drafting commands, never destructive or output commands", () => {
  for (const command of ["delete", "undo", "redo", "print_preview", "layout"] as const) expect(repeatableCommand(command)).toBe(false);
  for (const command of ["rectangle", "stretch", "fillet", "chamfer", "rectangular_array", "endpoint"] as const) {
    expect(repeatableCommand(command)).toBe(true);
  }
});

test("parameter changes retain picked geometry and reject invalid candidates", () => {
  const options = { ...initialDraftingOptions, distance: "300", angle: "90" };
  expect(retuneDraftOperation({ kind: "offset", entity_ids: ["line"], distance: -10 }, options)).toEqual({ kind: "offset", entity_ids: ["line"], distance: -300 });
  expect(retuneDraftOperation({ kind: "create", entity: { type: "line", layer: "0-1", p1: [10, 20], p2: [20, 20] } }, options)).toMatchObject({ kind: "create", entity: { p1: [10, 20] } });
  const rectangle = { kind: "rectangle" as const, layer: "0-1", p1: [10, 20] as [number, number], p2: [30, 40] as [number, number] };
  expect(retuneDraftOperation(rectangle, { ...options, width: "0" })).toBeNull();
  expect(retuneDraftOperation(rectangle, { ...options, width: "300", height: "200" })).toEqual({ ...rectangle, pen:null, p2: [310, 220] });
  expect(retuneDraftOperation({ ...rectangle, p2: [-10, -20] }, { ...options, width: "300", height: "200" })).toEqual({ ...rectangle, pen:null, p2: [-290, -180] });
  const scaling = { kind: "scale" as const, entity_ids: ["line"], center: [10, 20] as [number, number], factor: 2 };
  expect(retuneDraftOperation(scaling, { ...options, scale: "0" })).toBeNull();
  expect(retuneDraftOperation(scaling, { ...options, scale: "-2" })).toBeNull();
  expect(retuneDraftOperation(scaling, { ...options, scale: "3" })).toEqual({ ...scaling, factor: 3 });
  expect(retuneDraftOperation(rectangle, {...options,width:"300",height:"200",pen:"outline"})).toMatchObject({pen:"outline"});
  expect(retuneDraftOperation({kind:"create",entity:{type:"point",layer:"0-1",at:[10,20]}}, {...options,pen:"outline"})).toMatchObject({entity:{pen:"outline",at:[10,20]}});
  expect(retuneDraftOperation({kind:"create",entity:{type:"text",layer:"0-1",at:[10,20],style:"note",value:"old"}}, {...options,text:"室名\nA2",textWritingMode:"vertical_upright"})).toMatchObject({entity:{writing_mode:"vertical_upright",at:[10,20],value:"室名\nA2"}});
});
