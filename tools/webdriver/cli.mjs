/**
 * The automation endpoint from a shell (M71).
 *
 * The MCP server is the same client with a protocol around it; this is for a
 * person, and for an agent that has a shell but no MCP connection — a server
 * registered in `.mcp.json` is loaded when the client starts, so it is not
 * available in the session that added it.
 *
 *   node tools/webdriver/cli.mjs shot [name]     start the app, capture, stop
 *   node tools/webdriver/cli.mjs eval '<js>'     run a script, print the result
 *   node tools/webdriver/cli.mjs serve           start and hold, printing the port
 *
 * With `VE_DRIVER_PORT` set, every command talks to that endpoint instead of
 * starting one, so a held application can be driven repeatedly.
 */

import { connect, launch } from "./client.mjs";

const [command, ...rest] = process.argv.slice(2);
const held = process.env.VE_DRIVER_PORT;
const quiet = (line) => {
  if (process.env.VE_DRIVER_VERBOSE) process.stderr.write(`${line}\n`);
};

async function withDriver(run) {
  if (held) return run(connect(held), false);
  const driver = await launch({ onLog: quiet });
  try {
    return await run(driver, true);
  } finally {
    await driver.close();
  }
}

try {
  switch (command) {
    case "shot": {
      // `VE_DRIVER_OPEN` names a project to open first: a cold start is the
      // start screen and has no renderer to read a frame back from.
      const path = await withDriver(async (driver) => {
        await driver.ready();
        if (process.env.VE_DRIVER_OPEN) await driver.open(process.env.VE_DRIVER_OPEN);
        return driver.capture(rest[0] ?? "driver");
      });
      process.stdout.write(`${path}\n`);
      break;
    }
    case "eval": {
      if (!rest[0]) throw new Error("eval needs a script");
      const value = await withDriver(async (driver) => {
        await driver.ready();
        return driver.evaluate(rest[0]);
      });
      process.stdout.write(`${JSON.stringify(value, null, 2)}\n`);
      break;
    }
    case "serve": {
      const driver = await launch({ onLog: (line) => process.stderr.write(`${line}\n`) });
      process.stdout.write(`${driver.port}\n`);
      // Held until interrupted: the point is to leave it running.
      await new Promise(() => {});
      break;
    }
    default:
      process.stderr.write("usage: cli.mjs shot [name] | eval <js> | serve\n");
      process.exit(2);
  }
} catch (error) {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
  process.exit(1);
}
