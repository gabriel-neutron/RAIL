import { describe, expect, it } from "vitest";

import {
  parseContract,
  renderRust,
  renderTs,
} from "../../scripts/gen-ipc-events.mjs";

const valid = {
  EVENT_THING: {
    name: "thing",
    type: "Thing",
    doc: ["A thing."],
    fields: [{ name: "some_value", type: "u32" }],
  },
};

const withEvent = (key: string, entry: unknown) => ({ ...valid, [key]: entry });

// The generator is the only thing standing between the contract and both
// sides of the wire, so its rejections are load-bearing: a typo that slips
// through would ship a mismatched Rust/TS pair.
describe("parseContract", () => {
  it("accepts a well-formed contract", () => {
    expect(parseContract(valid)).toHaveLength(1);
  });

  it("rejects an unknown field type", () => {
    expect(() =>
      parseContract(
        withEvent("EVENT_OTHER", {
          name: "other",
          type: "Other",
          fields: [{ name: "x", type: "i128" }],
        }),
      ),
    ).toThrow(/unknown field type/);
  });

  it("rejects a duplicate wire name", () => {
    expect(() =>
      parseContract(
        withEvent("EVENT_DUPE", { name: "thing", type: "Dupe", fields: [] }),
      ),
    ).toThrow(/duplicate wire name/);
  });

  it("rejects a duplicate payload type", () => {
    expect(() =>
      parseContract(
        withEvent("EVENT_DUPE", { name: "dupe", type: "Thing", fields: [] }),
      ),
    ).toThrow(/duplicate payload type/);
  });

  it("rejects a non-kebab-case wire name", () => {
    expect(() =>
      parseContract(
        withEvent("EVENT_BAD", { name: "Bad_Name", type: "Bad", fields: [] }),
      ),
    ).toThrow(/kebab-case/);
  });

  it("rejects a non-snake_case field name", () => {
    expect(() =>
      parseContract(
        withEvent("EVENT_BAD", {
          name: "bad",
          type: "Bad",
          fields: [{ name: "someValue", type: "u32" }],
        }),
      ),
    ).toThrow(/snake_case/);
  });

  it("rejects a field that is both optional and nullable", () => {
    expect(() =>
      parseContract(
        withEvent("EVENT_BAD", {
          name: "bad",
          type: "Bad",
          fields: [
            { name: "x", type: "string", optional: true, nullable: true },
          ],
        }),
      ),
    ).toThrow(/exclusive/);
  });
});

describe("renderers", () => {
  it("camel-cases field names on the TypeScript side", () => {
    expect(renderTs(parseContract(valid))).toContain("someValue: number;");
  });

  it("keeps snake_case idents and derives Copy on the Rust side", () => {
    const rust = renderRust(parseContract(valid));
    expect(rust).toContain("pub some_value: u32,");
    expect(rust).toContain("#[derive(Debug, Clone, Copy, Serialize)]");
  });
});
