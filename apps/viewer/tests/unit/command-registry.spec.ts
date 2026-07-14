import { expect, test } from "@playwright/test";
import { commandForKeyboardEvent, commandLabel } from "../../src/command-registry";

test("maps Jw_cad-style command shortcuts", () => {
  const event = (key: string) => ({ key } as KeyboardEvent);
  expect(commandForKeyboardEvent(event("l"))).toBe("line");
  expect(commandForKeyboardEvent(event("Delete"))).toBe("delete");
  expect(commandForKeyboardEvent({ key: "z" } as KeyboardEvent)).toBe("undo");
  expect(commandForKeyboardEvent({ key: "z", shiftKey: true } as KeyboardEvent)).toBe("redo");
  expect(commandForKeyboardEvent({ key: "z", ctrlKey: true } as KeyboardEvent)).toBe("undo");
  expect(commandForKeyboardEvent(event("Escape"))).toBe("select");
  expect(commandForKeyboardEvent(event("unknown"))).toBeNull();
  expect(commandLabel("trim")).toBe("Trim");
});
