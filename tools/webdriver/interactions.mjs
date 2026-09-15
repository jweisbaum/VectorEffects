/** Exercise tool menus, first map gestures, inspector layout and timeline in WebKit. */
import { spawn } from "node:child_process";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { build } from "esbuild";
import { connect, portFromLine } from "./client.mjs";

const root = await mkdtemp(join(tmpdir(), "ve-interactions-"));
const bundle = await build({entryPoints:["tools/webdriver/interaction-fixture.tsx"],bundle:true,write:false,
  format:"iife",globalName:"Interactions",jsx:"automatic",define:{"import.meta.env.DEV":"false","process.env.NODE_ENV":'"production"'}});
const css = await readFile("ui/src/styles.css", "utf8");
const child = spawn(resolve("target/release/ve-app"), [], {
  env:{...process.env,VE_AUTOMATION_ROOT:root},stdio:["ignore","pipe","pipe"],
});
try {
  const port = await new Promise((done,reject) => {
    const timeout=setTimeout(() => reject(new Error("No automation port")),300_000);
    createInterface({input:child.stdout}).on("line",line => {const port=portFromLine(line);if(port!==null){clearTimeout(timeout);done(port);}});
    createInterface({input:child.stderr}).on("line",line => {if(line.includes("ERROR") || line.includes("panicked")) console.error(line);});
    child.once("exit",code => {clearTimeout(timeout);reject(new Error(`App exited ${code}`));});
  });
  const driver=connect(port); await driver.ready();
  await driver.evaluate(`const done=arguments[arguments.length-1]; window.__TAURI_INTERNALS__.invoke("new_project", {
    request:{name:"Interaction checks",field_kind:"wind",resolution:"1.0",step_hours:1,step_count:12},discardUnsaved:true
  }).then(done,e=>done({error:"Create",message:String(e)}));`);
  await driver.evaluate(`const done=arguments[arguments.length-1]; ${bundle.outputFiles[0].text}
    Interactions.install(arguments[0]); window.__interactionResult=null;
    Interactions.check(arguments[0]).then(r=>window.__interactionResult=r,e=>window.__interactionResult={error:String(e)}); done(true);`, [css]);
  let report=null;
  for(let i=0;i<180 && !report;i++) {
    await new Promise(resolve=>setTimeout(resolve,1000));
    report=await driver.evaluate("arguments[arguments.length-1](window.__interactionResult)");
  }
  if(report?.circleImage) {
    await writeFile(join(root,"circle-properties.png"),Buffer.from(report.circleImage.split(",")[1],"base64"));
    delete report.circleImage;
  }
  await writeFile(join(root,"report.json"),JSON.stringify(report,null,2));
  console.log(JSON.stringify({root,...report}));
  if(!report || report.error) throw new Error(report?.error ?? "Timed out");
} finally {child.kill("SIGTERM");}
