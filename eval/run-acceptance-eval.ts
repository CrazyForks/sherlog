#!/usr/bin/env -S node --import tsx

import { runAcceptanceGate } from "./acceptance-gate";

const argv = process.argv.slice(2);
if (argv.includes("--help") || argv.includes("-h")) {
  console.log([
    "Usage: npm run eval:acceptance -- [--keep-temp] [--require-candidate] [--cli-argv-json '<json-array>']",
    "",
    "By default the checkout TypeScript CLI is tested.",
    "Set SHLOG_BIN_UNDER_TEST to test one executable, or use --cli-argv-json",
    "for an executable prefix with fixed arguments. Explicit argv JSON wins.",
  ].join("\n"));
  process.exit(0);
}

const keepTemp = argv.includes("--keep-temp");
const cliArgvJson = optionValue(argv, "--cli-argv-json");
const result = await runAcceptanceGate({
  keepTemp,
  requireCandidateOverride: argv.includes("--require-candidate"),
  ...(cliArgvJson ? { cliArgvJson } : {}),
});

console.log(JSON.stringify(result, null, 2));
if (result.scoreboard.hardFail > 0) process.exitCode = 1;

function optionValue(argv: string[], name: string): string | undefined {
  const index = argv.indexOf(name);
  if (index < 0) return undefined;
  const value = argv[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a JSON string array`);
  return value;
}
