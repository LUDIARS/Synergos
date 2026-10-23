// Excubitor の data-import 入口: --input <Taildrop 上の絶対パス> --sha256 <hex>。
// 停止中に、空のデータディレクトリへだけ復元する (既存データがあれば拒否)。
import { readArgs } from './lib/args.mjs';
import { readBundle } from './lib/dir-bundle.mjs';
import { dataDir, SERVICE } from './lib/paths.mjs';

try {
  const { input, sha256 } = readArgs(process.argv.slice(2), ['input', 'sha256']);
  const summary = await readBundle(input, sha256, dataDir(), SERVICE);
  console.log(`import: ${summary.files} files restored`);
} catch (error) {
  console.error(`import: ${error.message}`);
  process.exit(1);
}
