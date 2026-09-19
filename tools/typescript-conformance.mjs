// Development-only comparison with the pinned SDK parser and its independent jomini oracle.
import { createRequire } from 'node:module';
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { spawn, execFileSync } from 'node:child_process';
import { createInterface } from 'node:readline';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import assert from 'node:assert/strict';

const sdk = resolve(process.env.PDX_SDK_PATH ?? '../pdx-sdk');
const sourceRevision = '400e92f00afb524371198bedd4f9cddfcab48f78';
assert.equal(execFileSync('git', ['rev-parse', 'HEAD'], { cwd:sdk, encoding:'utf8' }).trim(), sourceRevision, 'SDK revision changed: review the oracle before repinning');
assert.equal(execFileSync('git', ['status', '--porcelain', '--', 'packages/pdxscript'], { cwd:sdk, encoding:'utf8' }).trim(), '', 'SDK parser must be unchanged');
const api = await import(pathToFileURL(join(sdk, 'packages/pdxscript/src/index.ts')));
const require = createRequire(join(sdk, 'package.json'));
const ts = require('typescript');
const child = spawn('./target/debug/examples/conformance', [], { stdio:['pipe','pipe','inherit'] });
const lines = createInterface({ input:child.stdout })[Symbol.asyncIterator]();
let count = 0;
const fixtures = [];
async function compare(source, file) {
  let expected;
  try {
    const doc = api.parse(source, file);
    expected = { items:api.withoutLines(doc.items), canonical:api.serialize(doc.items), diagnostics:doc.diagnostics.map(({kind,line,text}) => ({kind,line,text})) };
  } catch(e) {
    if (!(e instanceof api.PdxSyntaxError)) throw e;
    expected = { error:true, line:e.line };
  }
  child.stdin.write(JSON.stringify({source,file})+'\n');
  const line = await lines.next();
  assert(!line.done, 'Rust runner stopped');
  assert.deepEqual(JSON.parse(line.value), expected, file + ': ' + source.slice(0,120));
  count++;
  if (!process.argv.includes('--vanilla')) fixtures.push({source,file,expected});
}
try {
  for (const name of ['parser.test.ts','region-fallback.test.ts','walk.test.ts','properties.test.ts']) {
    const source = await readFile(join(sdk,'packages/pdxscript/tests',name),'utf8');
    const tree = ts.createSourceFile(name, source, ts.ScriptTarget.Latest, true);
    const cases = [];
    function visit(node) {
      if (ts.isCallExpression(node) && ['parse','clean','first','value','emitted','expectFixpoint'].includes(node.expression.getText(tree)) && node.arguments[0] && (ts.isStringLiteral(node.arguments[0]) || ts.isNoSubstitutionTemplateLiteral(node.arguments[0]))) {
        cases.push(node.arguments[0].text);
      }
      ts.forEachChild(node, visit);
    }
    visit(tree);
    for (let i=0;i<cases.length;i++) await compare(cases[i], `${name}:${i}`);
  }
  if (process.argv.includes('--vanilla')) {
    const install = process.env.STELLARIS_PATH;
    assert(install, 'STELLARIS_PATH is required; corpus checks cannot skip');
    async function walk(dir) {
      const result = [];
      for (const entry of await readdir(dir,{withFileTypes:true})) {
        const path = join(dir,entry.name);
        if(entry.isDirectory()) result.push(...await walk(path));
        else if(entry.name.endsWith('.txt') && !['HOW_TO_MAKE_NEW_SHIPS.txt','99_README_EDICTS.txt'].includes(entry.name)) result.push(path);
      }
      return result.sort();
    }
    const files = await walk(join(install,'common'));
    assert(files.length > 500, 'Corpus is missing or unexpectedly small');
    for(const file of files) await compare(await readFile(file,'utf8'),file);
    // The source suite pins exact independent-oracle differences and refuses unexpected drift.
    execFileSync(process.execPath, ['--conditions=pdx-source', join(sdk,'node_modules/vitest/vitest.mjs'), 'run', 'packages/pdxscript/tests/corpus.test.ts', 'packages/pdxscript/tests/differential.test.ts'], { cwd:sdk, env:{...process.env,PDX_REQUIRE_INSTALL:'1'}, stdio:'inherit' });
  }
  if(process.argv.includes('--write-fixtures')) await writeFile('tests/typescript-fixtures.json',JSON.stringify({sourceRevision,cases:fixtures},null,2)+'\n');
  console.log(`TypeScript/Rust conformance: ${count} inputs agree (trees, repairs, errors, canonical bytes).`);
} finally { child.stdin.end(); }
