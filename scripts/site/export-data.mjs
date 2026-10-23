// Excubitor の data-export 入口: --output <絶対パス>。
// 停止中の synergos-control のデータ (レジストリ JSON と custody の保管物) を単一 bundle にする。
// control.toml は起動のたびに環境変数から作り直すので含めない。
import { readArgs } from './lib/args.mjs';
import { writeBundle } from './lib/dir-bundle.mjs';
import { dataDir, SERVICE } from './lib/paths.mjs';

try {
  const { output } = readArgs(process.argv.slice(2), ['output']);
  const summary = await writeBundle(dataDir(), output, SERVICE, { exclude: ['control.toml'] });
  console.log(`export: ${summary.files} files, ${summary.bytes} bytes`);
} catch (error) {
  console.error(`export: ${error.message}`);
  process.exit(1);
}
