// golden vector (spec/vectors/*.json)。TS 実装 (Curare / Cernere) と同じファイルで照合する。
#pragma once
#include <string>

namespace synergos::trust::test {

/// 決まった seed から vector を作り直して dir へ書く (`synergos_trust_test --write-vectors`)。
/// 各ケースの意図した結果 (成功 / エラーコード) と実装の結果が食い違えば例外。
void write_vectors(const std::string& dir);

/// dir の vector を読み、入力から実装で結果を求めてファイルの期待値と照合する。食い違えば例外。
void check_vectors(const std::string& dir);

} // namespace synergos::trust::test
