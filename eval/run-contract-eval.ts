#!/usr/bin/env -S node --import tsx

import { runContractGate } from "./contract-gate";

const argv = process.argv.slice(2);
if (argv.includes("--help") || argv.includes("-h")) {
  console.log([
    "Usage: npm run eval:contract -- [options]",
    "",
    "Options:",
    "  --reference-argv-json '<json-array>'  Override the TypeScript reference command prefix",
    "  --candidate-argv-json '<json-array>'  Override the candidate command prefix",
    "  --require-candidate                   Fail instead of falling back to the TypeScript candidate",
    "  --keep-temp                           Keep fixture and database directories",
    "",
    "The reference defaults to this checkout's TypeScript CLI. The candidate",
    "uses explicit argv JSON first, then SHLOG_CLI_ARGV_JSON, then",
    "SHLOG_BIN_UNDER_TEST, and finally the same TypeScript CLI.",
  ].join("\n"));
  process.exit(0);
}

const referenceArgvJson = optionValue(argv, "--reference-argv-json");
const candidateArgvJson = optionValue(argv, "--candidate-argv-json");
const result = await runContractGate({
  keepTemp: argv.includes("--keep-temp"),
  requireCandidateOverride: argv.includes("--require-candidate"),
  ...(referenceArgvJson !== undefined ? { referenceArgvJson } : {}),
  ...(candidateArgvJson !== undefined ? { candidateArgvJson } : {}),
});

console.log(JSON.stringify(result, null, 2));
if (result.failed > 0) process.exitCode = 1;

function optionValue(args: string[], name: string): string | undefined {
  const index = args.indexOf(name);
  if (index < 0) return undefined;
  const value = args[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a JSON string array`);
  return value;
}
