import { describe, expect, test } from "vitest";
import {
  parseCliArgvJson,
  resolveCliUnderTest,
  runCliUnderTest,
} from "./cli-under-test";

describe("CLI under test", () => {
  test("defaults to the checkout TypeScript reference", () => {
    const cli = resolveCliUnderTest({ env: {} });

    expect(cli.source).toBe("typescript-reference");
    expect(cli.argv.slice(0, 4)).toEqual([
      process.execPath,
      "--disable-warning=ExperimentalWarning",
      "--import",
      "tsx",
    ]);
    expect(cli.argv[4]).toMatch(/\/src\/cli\.ts$/);
  });

  test("uses explicit argv JSON before SHLOG_BIN_UNDER_TEST without shell splitting", () => {
    const cli = resolveCliUnderTest({
      argvJson: JSON.stringify(["cargo", "run", "--quiet", "--"]),
      env: { SHLOG_BIN_UNDER_TEST: "/tmp/shlog candidate" },
    });

    expect(cli).toEqual({
      source: "argv-json",
      argv: ["cargo", "run", "--quiet", "--"],
    });

    expect(resolveCliUnderTest({ env: { SHLOG_BIN_UNDER_TEST: "/tmp/shlog candidate" } })).toEqual({
      source: "env-bin",
      argv: ["/tmp/shlog candidate"],
    });
  });

  test("rejects ambiguous argv JSON", () => {
    expect(() => parseCliArgvJson("not json")).toThrow("non-empty string array");
    expect(() => parseCliArgvJson("[]")).toThrow("non-empty string array");
    expect(() => parseCliArgvJson('["shlog", 1]')).toThrow("non-empty string array");
    expect(() => parseCliArgvJson('["shlog", ""]')).toThrow("non-empty string array");
  });

  test("appends command arguments to an explicit executable prefix", async () => {
    const script = "process.stdout.write(JSON.stringify(process.argv.slice(1)))";
    const cli = resolveCliUnderTest({
      argvJson: JSON.stringify([process.execPath, "--eval", script]),
      env: {},
    });

    const result = await runCliUnderTest(cli, ["find", "needle", "--json"]);

    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe("");
    expect(JSON.parse(result.stdout)).toEqual(["find", "needle", "--json"]);
  });
});
