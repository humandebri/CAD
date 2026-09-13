import { expect, test } from "@playwright/test";
import { CAD_COMMANDS, commandForKeyboardEvent, commandFromText, parseCoordinateInput } from "../../src/command-registry";

test("maps Jw_cad-style command shortcuts", () => {
  const event = (key: string) => ({ key } as KeyboardEvent);
  expect(commandForKeyboardEvent(event("l"))).toBe("line");
  expect(commandForKeyboardEvent(event("Delete"))).toBe("delete");
  expect(commandForKeyboardEvent({ key: "z" } as KeyboardEvent)).toBe("undo");
  expect(commandForKeyboardEvent({ key: "z", shiftKey: true } as KeyboardEvent)).toBe("redo");
  expect(commandForKeyboardEvent({ key: "z", ctrlKey: true } as KeyboardEvent)).toBe("undo");
  expect(commandForKeyboardEvent(event("Escape"))).toBe("select");
  expect(commandForKeyboardEvent(event("unknown"))).toBeNull();
});

test("parses command-bar commands and CAD coordinates", () => {
  for (const command of CAD_COMMANDS) {
    expect(commandFromText(command)).toBe(command);
    expect(commandFromText(command.replaceAll("_", " "))).toBe(command);
  }
  expect(commandFromText("LINE")).toBe("line");
  expect(parseCoordinateInput("10,20", null)).toEqual([10, 20]);
  expect(parseCoordinateInput("@5,-2", [10, 20])).toEqual([15, 18]);
  const polar = parseCoordinateInput("@10<90", [1, 2]);
  expect(polar?.[0]).toBeCloseTo(1);
  expect(polar?.[1]).toBeCloseTo(12);
  expect(parseCoordinateInput("@5,2", null)).toBeNull();
  expect(parseCoordinateInput("bad", [0, 0])).toBeNull();
});
