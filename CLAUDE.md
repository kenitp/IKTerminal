# IkTerminal

Rust 製の軽量ターミナル。仕様は `docs/00_spec/`、設計は `docs/01_design/` を参照。

## コマンド

- ビルド: `cargo build` / `cargo build --release`
- 検査: `cargo build --all-targets`、`cargo build --release`、`cargo clippy --all-targets -- -D warnings`。ビルド警告は禁止。
- テスト: `cargo test`
- インストーラ: `.\scripts\build-installer.ps1`
- Linux パッケージ: `./scripts/package-linux.sh`

## ルール

- 実装を変更したら `docs/00_spec` と `docs/01_design` を同時に更新する。
- レイヤーの依存方向 (`app` -> `ui` -> `session` -> `backend` -> `terminal`) を守る。`backend` から `ui` を参照しない。
- UI コンポーネントは結果を `*Action` / `*Result` で返し、状態変更は `app` で行う。
- 依存クレートの追加はメモリ使用量とバイナリサイズへの影響を確認してから行う。
- 冗長なコード、未使用コード、変更履歴を書いたコメントを残さない。
- 絵文字を使わない。
