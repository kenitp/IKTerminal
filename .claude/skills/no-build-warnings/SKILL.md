---
name: no-build-warnings
description: >-
  IkTerminal のビルド警告をゼロにする。cargo build、cargo build --release、
  cargo clippy の warning を確認し、残さず直す。実装変更後、ビルド、警告、
  clippy、リリースビルドの確認時に使う。
---

# ビルド警告ゼロ

ビルドワーニングは禁止です。このクレートの `warning` は失敗と同じ。`allow` で隠さない。依存クレートの警告は対象外。

実装を変えたら、完了前に次を実行する。

1. `cargo build --all-targets`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo build --release`

debug と release で `cfg` が違うコードは、両方で警告が出ないようにする。テストからだけ使う関数は `#[cfg(test)]` に入れ、release のバイナリに残さない。警告が出たら直してから同じコマンドをやり直す。
