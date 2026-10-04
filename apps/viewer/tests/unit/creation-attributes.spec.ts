import { test, expect } from "@playwright/test";
import { acquireCreationAttributes } from "../../src/creation-attributes";
import type { EditorDrawingState, EditorEntity, LayerWorkspaceState } from "../../src/artifacts";

const editor = {pens:["outline"],text_styles:["note"],dimension_styles:["dim"],fills:["black"]} as EditorDrawingState;
const layers = {layers:[{id:"0-1"}]} as LayerWorkspaceState;
const entity = (type: string, extra: Record<string,unknown> = {}): EditorEntity => ({schema_version:"0.3",id:"source",layer:"0-1",type,...extra});

test("acquire references by entity type without copying geometry or a dimension style into text", () => {
  expect(acquireCreationAttributes(entity("text",{pen:"outline",style:"note",value:"original",at:[10,20]}),editor,layers)).toEqual({sourceId:"source",layer:"0-1",pen:"outline",textStyle:"note",dimensionStyle:null,fill:null});
  expect(acquireCreationAttributes(entity("dimension",{style:"dim"}),editor,layers)).toMatchObject({pen:null,textStyle:null,dimensionStyle:"dim",fill:null});
  expect(acquireCreationAttributes(entity("hatch",{fill:"black"}),editor,layers)).toMatchObject({fill:"black",textStyle:null});
  expect(acquireCreationAttributes(entity("line"),editor,layers)).toMatchObject({pen:null});
  for (const invalid of [entity("line",{pen:"missing"}),entity("text",{style:"missing"}),entity("dimension",{style:"note"}),entity("solid",{fill:"missing"}),entity("line",{layer:"missing"})]) {
    expect(()=>acquireCreationAttributes(invalid,editor,layers)).toThrow(/unavailable/);
  }
});
