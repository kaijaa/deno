#!/usr/bin/env -S deno run --reload --allow-run
// Copyright 2018-2020 the Deno authors. All rights reserved. MIT license.
import "./unit_tests.ts";
import {
  readLines,
  permissionCombinations,
  Permissions,
  registerUnitTests,
  fmtPerms,
  parseArgs,
  reportToConn,
  createResolvable,
} from "./test_util.ts";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const reportToConsole = (Deno as any)[Deno.symbols.internal]
  .reportToConsole as (message: Deno.TestMessage) => void;

interface PermissionSetTestResult {
  perms: Permissions;
  passed: boolean;
  endMessage: Deno.TestMessage["end"];
  permsStr: string;
}

const PERMISSIONS: Deno.PermissionName[] = [
  "read",
  "write",
  "net",
  "env",
  "run",
  "plugin",
  "hrtime",
];

/**
 * Take a list of permissions and revoke missing permissions.
 */
async function dropWorkerPermissions(
  requiredPermissions: Deno.PermissionName[]
): Promise<void> {
  const permsToDrop = PERMISSIONS.filter((p): boolean => {
    return !requiredPermissions.includes(p);
  });

  for (const perm of permsToDrop) {
    await Deno.permissions.revoke({ name: perm });
  }
}

async function workerRunnerMain(
  addrStr: string,
  permsStr: string,
  filter?: string
): Promise<void> {
  const [hostname, port] = addrStr.split(":");
  const addr = { hostname, port: Number(port) };

  let perms: Deno.PermissionName[] = [];
  if (permsStr.length > 0) {
    perms = permsStr.split(",") as Deno.PermissionName[];
  }
  // Setup reporter
  const conn = await Deno.connect(addr);
  // Drop current process permissions to requested set
  await dropWorkerPermissions(perms);
  // Register unit tests that match process permissions
  await registerUnitTests();
  // Execute tests
  await Deno.runTests({
    exitOnFail: false,
    filter,
    reportToConsole: false,
    onMessage: reportToConn.bind(null, conn),
  });
}

function spawnWorkerRunner(
  verbose: boolean,
  addr: string,
  perms: Permissions,
  filter?: string
): Deno.Process {
  // run subsequent tests using same deno executable
  const permStr = Object.keys(perms)
    .filter((permName): boolean => {
      return perms[permName as Deno.PermissionName] === true;
    })
    .join(",");

  const cmd = [
    Deno.execPath(),
    "run",
    "-A",
    "cli/js/tests/unit_test_runner.ts",
    "--worker",
    `--addr=${addr}`,
    `--perms=${permStr}`,
  ];

  if (filter) {
    cmd.push("--");
    cmd.push(filter);
  }

  const ioMode = verbose ? "inherit" : "null";

  const p = Deno.run({
    cmd,
    stdin: ioMode,
    stdout: ioMode,
    stderr: ioMode,
  });

  return p;
}

async function runTestsForPermissionSet(
  listener: Deno.Listener,
  addrStr: string,
  verbose: boolean,
  perms: Permissions,
  filter?: string
): Promise<PermissionSetTestResult> {
  const permsFmt = fmtPerms(perms);
  console.log(`Running tests for: ${permsFmt}`);
  const workerProcess = spawnWorkerRunner(verbose, addrStr, perms, filter);
  // Wait for worker subprocess to go online
  const conn = await listener.accept();

  let expectedPassedTests;
  let endMessage: Deno.TestMessage["end"];

  try {
    for await (const line of readLines(conn)) {
      const message = JSON.parse(line) as Deno.TestMessage;
      reportToConsole(message);
      if (message.start != null) {
        expectedPassedTests = message.start.tests.length;
      } else if (message.end != null) {
        endMessage = message.end;
      }
    }
  } finally {
    // Close socket to worker.
    conn.close();
  }

  if (expectedPassedTests == null) {
    throw new Error("Worker runner didn't report start");
  }

  if (endMessage == null) {
    throw new Error("Worker runner didn't report end");
  }

  const workerStatus = await workerProcess.status();
  if (!workerStatus.success) {
    throw new Error(
      `Worker runner exited with status code: ${workerStatus.code}`
    );
  }

  workerProcess.close();

  const passed = expectedPassedTests === endMessage.passed + endMessage.ignored;

  return {
    perms,
    passed,
    permsStr: permsFmt,
    endMessage,
  };
}

async function runTestsInThreadForPermissionSet(
  verbose: boolean,
  perms: Permissions,
  filter?: string
): Promise<PermissionSetTestResult> {
  const permsFmt = fmtPerms(perms);
  console.log(`Running tests for: ${permsFmt}`);

  const permStr = Object.keys(perms)
    .filter((permName): boolean => {
      return perms[permName as Deno.PermissionName] === true;
    })
    .join(",");

  const resolvable = createResolvable();

  const worker = new Worker("./unit_test_worker.ts", {
    type: "module",
    name: permStr,
    deno: true,
    permissions: perms,
  });

  let expectedPassedTests;
  let endMessage: Deno.TestMessage["end"];

  worker.postMessage({
    cmd: "run",
    filter: filter,
    verbose: verbose,
  });

  // @ts-ignore
  worker.onmessage = function(e): void {
    const message = JSON.parse(e.data.testMsg) as Deno.TestMessage;
    reportToConsole(message);
    if (message.start != null) {
      expectedPassedTests = message.start.tests.length;
    } else if (message.end != null) {
      endMessage = message.end;
      worker.terminate();
      resolvable.resolve();
    }
  };

  await resolvable;

  if (expectedPassedTests == null) {
    throw new Error("Worker runner didn't report start");
  }

  if (endMessage == null) {
    throw new Error("Worker runner didn't report end");
  }

  const passed = expectedPassedTests === endMessage.passed + endMessage.ignored;

  return {
    perms,
    passed,
    permsStr: permsFmt,
    endMessage,
  };
}

async function threadedRunnerMain(
  verbose: boolean,
  filter?: string
): Promise<void> {
  console.log(
    "Discovered permission combinations for tests:",
    permissionCombinations.size
  );

  for (const perms of permissionCombinations.values()) {
    console.log("\t" + fmtPerms(perms));
  }

  const testResults = new Set<PermissionSetTestResult>();

  for (const perms of permissionCombinations.values()) {
    const result = await runTestsInThreadForPermissionSet(
      verbose,
      perms,
      filter
    );
    testResults.add(result);
  }

  // if any run tests returned non-zero status then whole test
  // run should fail
  let testsPassed = true;

  for (const testResult of testResults) {
    const { permsStr, endMessage } = testResult;
    console.log(`Summary for ${permsStr}`);
    reportToConsole({ end: endMessage });
    testsPassed = testsPassed && testResult.passed;
  }

  if (!testsPassed) {
    console.error("Unit tests failed");
    Deno.exit(1);
  }

  console.log("Unit tests passed");
}

async function masterRunnerMain(
  verbose: boolean,
  filter?: string
): Promise<void> {
  console.log(
    "Discovered permission combinations for tests:",
    permissionCombinations.size
  );

  for (const perms of permissionCombinations.values()) {
    console.log("\t" + fmtPerms(perms));
  }

  const testResults = new Set<PermissionSetTestResult>();
  const addr = { hostname: "127.0.0.1", port: 4510 };
  const addrStr = `${addr.hostname}:${addr.port}`;
  const listener = Deno.listen(addr);

  for (const perms of permissionCombinations.values()) {
    const result = await runTestsForPermissionSet(
      listener,
      addrStr,
      verbose,
      perms,
      filter
    );
    testResults.add(result);
  }

  // if any run tests returned non-zero status then whole test
  // run should fail
  let testsPassed = true;

  for (const testResult of testResults) {
    const { permsStr, endMessage } = testResult;
    console.log(`Summary for ${permsStr}`);
    reportToConsole({ end: endMessage });
    testsPassed = testsPassed && testResult.passed;
  }

  if (!testsPassed) {
    console.error("Unit tests failed");
    Deno.exit(1);
  }

  console.log("Unit tests passed");
}

const HELP = `Unit test runner

Run tests matching current process permissions:

  deno --allow-write unit_test_runner.ts

  deno --allow-net --allow-hrtime unit_test_runner.ts

  deno --allow-write unit_test_runner.ts -- testWriteFile

Run "master" process that creates "worker" processes
for each discovered permission combination:

  deno -A unit_test_runner.ts --master

Run worker process for given permissions:

  deno -A unit_test_runner.ts --worker --perms=net,read,write --addr=127.0.0.1:4500


OPTIONS:
  --master
    Run in master mode, spawning worker processes for
    each discovered permission combination

  --worker
    Run in worker mode, requires "perms" and "addr" flags,
    should be run with "-A" flag; after setup worker will
    drop permissions to required set specified in "perms"

  --perms=<perm_name>...
    Set of permissions this process should run tests with,

  --addr=<addr>
    Address of TCP socket for reporting

ARGS:
  -- <filter>...
    Run only tests with names matching filter, must
    be used after "--"
`;

function assertOrHelp(expr: unknown): asserts expr {
  if (!expr) {
    console.log(HELP);
    Deno.exit(1);
  }
}

async function main(): Promise<void> {
  const args = parseArgs(Deno.args, {
    boolean: ["master", "worker", "threaded", "verbose"],
    "--": true,
  });

  if (args.help) {
    console.log(HELP);
    return;
  }

  const filter = args["--"][0];

  if (args.threaded) {
    return threadedRunnerMain(args.verbose, filter);
  }

  // Master mode
  if (args.master) {
    return masterRunnerMain(args.verbose, filter);
  }

  // Worker mode
  if (args.worker) {
    assertOrHelp(typeof args.addr === "string");
    assertOrHelp(typeof args.perms === "string");
    return workerRunnerMain(args.addr, args.perms, filter);
  }

  // Running tests matching current process permissions
  await registerUnitTests();
  await Deno.runTests({ filter });
}

main();
