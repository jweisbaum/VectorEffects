/** Isolated native Settings check; never installs plugins into the person's hosts. */
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { launch } from './client.mjs';
const root = await mkdtemp(join(tmpdir(), 've-assistant-ui-'));
let driver;
try {
  driver = await launch({ timeoutMs: 1800000, env: { VE_AUTOMATION_ROOT: join(root, 'app'), CODEX_HOME: join(root, 'codex'), CLAUDE_CONFIG_DIR: join(root, 'claude') }, onLog: line => process.stderr.write(line + '\n') });
  await driver.ready();
  const evaluate = script => driver.evaluate(`var done = arguments[arguments.length - 1]; ${script}`);
  const until = async expression => {
    const end = Date.now() + 30000;
    while (!(await evaluate(`done(${expression});`))) {
      if (Date.now() > end) throw new Error(`Timed out: ${expression}`);
      await new Promise(resolve => setTimeout(resolve, 100));
    }
  };
  await until(`document.querySelector('[data-feature="start:new"] button.primary') !== null`);
  await evaluate(`document.querySelector('[data-feature="start:new"] button.primary').click(); done(true);`);
  await until(`document.querySelector('[data-feature="shell:settings"]') !== null`);
  await evaluate(`document.querySelector('[data-feature="shell:settings"]').click(); done(true);`);
  await until(`document.querySelectorAll('[data-feature="settings:mcp"] strong').length === 3`);
  assert.equal(await evaluate(`done(document.querySelector('[data-feature="settings:mcp"] select').value);`), 'workflow');
  assert.equal(await evaluate(`done(document.querySelector('[data-feature="settings:mcp"] details').open);`), false);
  await evaluate(`document.querySelector('[data-feature="settings:mcp"]').scrollIntoView({block:'start'}); done(true);`);
  // Preserve the modal's scroll position and live form values. The raw endpoint
  // serializes scroll containers at their initial position, hiding this section.
  const data = await evaluate(`(async () => {
    const source = document.body, copy = source.cloneNode(true);
    const live = [source, ...source.querySelectorAll('*')];
    const clones = [copy, ...copy.querySelectorAll('*')];
    live.forEach((el, i) => {
      const clone = clones[i], style = getComputedStyle(el);
      for (const key of style) clone.style.setProperty(key, style.getPropertyValue(key));
      if (el instanceof HTMLInputElement) {
        clone.setAttribute('value', el.value);
        if (el.checked) clone.setAttribute('checked', ''); else clone.removeAttribute('checked');
      }
      if (el instanceof HTMLOptionElement) {
        if (el.selected) clone.setAttribute('selected', ''); else clone.removeAttribute('selected');
      }
    });
    live.forEach((el, i) => {
      if (!el.scrollTop && !el.scrollLeft) return;
      const clone = clones[i], wrap = document.createElement('div');
      const style = getComputedStyle(el);
      for (const key of ['display', 'flex-direction', 'flex-wrap', 'gap', 'align-items', 'justify-content', 'grid-template-columns', 'grid-template-rows'])
        wrap.style.setProperty(key, style.getPropertyValue(key));
      wrap.style.position = 'relative'; wrap.style.top = -el.scrollTop + 'px'; wrap.style.left = -el.scrollLeft + 'px';
      while (clone.firstChild) wrap.appendChild(clone.firstChild);
      clone.appendChild(wrap); clone.style.overflow = 'hidden';
    });
    copy.querySelectorAll('script,link').forEach(el => el.remove());
    const width = innerWidth, height = innerHeight;
    const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="' + width + '" height="' + height + '"><foreignObject width="100%" height="100%">' + new XMLSerializer().serializeToString(copy) + '</foreignObject></svg>';
    const picture = new Image(); picture.src = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
    await picture.decode();
    const canvas = document.createElement('canvas'); canvas.width = width; canvas.height = height;
    canvas.getContext('2d').drawImage(picture, 0, 0);
    return canvas.toDataURL('image/png').split(',')[1];
  })().then(done, error => done({error:'Screenshot',message:String(error)}));`);
  assert.equal(typeof data, 'string');
  const folder = join(process.cwd(), 'target/ux-shots/assistant-settings');
  await mkdir(folder, { recursive: true });
  const file = join(folder, 'connections.png');
  await writeFile(file, Buffer.from(data, 'base64'));
  console.log(`Native connection controls and default policy passed. Screenshot: ${file}`);
} finally {
  await driver?.close();
  await rm(root, { recursive: true, force: true });
}
