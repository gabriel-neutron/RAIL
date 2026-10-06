import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

/**
 * `App.css` is layout and structure only; `theme.css` owns the palette and the
 * type scale. See CONVENTIONS.md §2 Styling. This guard keeps the split from
 * rotting back into a second palette.
 *
 * Structural keywords are not a palette: `transparent`, `none`, `currentColor`
 * and the CSS-wide keywords reset user-agent chrome (native button borders and
 * backgrounds) without picking a colour, so they stay allowed.
 */
const STRUCTURAL_KEYWORDS = new Set([
  "transparent",
  "none",
  "currentcolor",
  "inherit",
  "initial",
  "unset",
  "revert",
]);

const NAMED_COLOURS = [
  "black",
  "white",
  "red",
  "green",
  "blue",
  "yellow",
  "orange",
  "purple",
  "pink",
  "brown",
  "grey",
  "gray",
  "cyan",
  "magenta",
  "silver",
  "gold",
  "teal",
  "navy",
  "olive",
  "maroon",
  "lime",
  "aqua",
  "fuchsia",
  "indigo",
  "violet",
  "beige",
  "ivory",
  "khaki",
  "salmon",
  "coral",
  "crimson",
  "tomato",
  "orchid",
  "plum",
  "tan",
  "wheat",
  "azure",
  "lavender",
  "linen",
  "snow",
];

const APP_CSS_PATH = join(process.cwd(), "src", "App.css");

function stripComments(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, "");
}

interface Declaration {
  property: string;
  value: string;
  line: number;
}

function readDeclarations(): Declaration[] {
  const lines = stripComments(readFileSync(APP_CSS_PATH, "utf8")).split("\n");
  const declarations: Declaration[] = [];

  lines.forEach((text, index) => {
    const match = /^\s*([-a-zA-Z]+)\s*:\s*([^;]+);/.exec(text);
    if (!match) return;
    declarations.push({
      property: match[1].toLowerCase(),
      value: match[2].trim(),
      line: index + 1,
    });
  });

  return declarations;
}

function isStructural(value: string): boolean {
  return STRUCTURAL_KEYWORDS.has(value.toLowerCase());
}

function describeAll(hits: Declaration[]): string {
  return hits.map((hit) => `App.css:${hit.line} ${hit.property}: ${hit.value}`).join("\n");
}

describe("App.css carries no palette", () => {
  const declarations = readDeclarations();

  it("parses App.css into declarations", () => {
    expect(declarations.length).toBeGreaterThan(0);
  });

  it("declares no hex colour literal", () => {
    const hits = declarations.filter((declaration) => /#[0-9a-fA-F]{3,8}\b/.test(declaration.value));
    expect(describeAll(hits)).toBe("");
  });

  it("declares no rgb/rgba/hsl/hsla colour function", () => {
    const hits = declarations.filter((declaration) =>
      /\b(rgba?|hsla?)\s*\(/i.test(declaration.value),
    );
    expect(describeAll(hits)).toBe("");
  });

  it("declares no named colour", () => {
    const pattern = new RegExp(`\\b(${NAMED_COLOURS.join("|")})\\b`, "i");
    const hits = declarations.filter(
      (declaration) => !isStructural(declaration.value) && pattern.test(declaration.value),
    );
    expect(describeAll(hits)).toBe("");
  });

  it("declares no font-family", () => {
    const hits = declarations.filter((declaration) => declaration.property === "font-family");
    expect(describeAll(hits)).toBe("");
  });

  it("declares no font-size", () => {
    const hits = declarations.filter((declaration) => declaration.property === "font-size");
    expect(describeAll(hits)).toBe("");
  });

  it("still allows structural keyword resets", () => {
    const keywordResets = declarations.filter(
      (declaration) =>
        (declaration.property === "background" ||
          declaration.property === "border" ||
          declaration.property === "outline") &&
        isStructural(declaration.value),
    );
    expect(keywordResets.length).toBeGreaterThan(0);
  });
});
