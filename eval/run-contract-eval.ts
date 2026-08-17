#!/usr/bin/env -S node --import tsx

import { runContractGate } from "./contract-gate";

const argv = process.argv.slice(2);
if (argv.includes("--help") || argv.includes("-h")) {
  console.log([
    "Usage: npm run eval:contract -- [options]",
    "",
    "Options:",
    "  --reference-argv-json '<json-array>'  Override the reference command prefix",
    "  --candidate-argv-json '<json-array>'  Override the candidate command prefix",
    "  --require-candidate                   Fail unless an explicit candidate is set",
    "  --keep-temp                           Keep fixture and database directories",
    "",
    "Both sides default to checkout target/release/shlog (else debug).",
    "If only the candidate is overridden and no checkout binary exists,",
    "the reference uses the same candidate (isolated state dirs still differ).",
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
