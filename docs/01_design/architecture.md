# IkTerminal アーキテクチャ設計

## 1. 技術選定

| 用途 | 採用 | 理由 |
| --- | --- | --- |
| GUI | eframe / egui 0.35 (Glow) | 即時モードで状態管理が単純。wgpu より大幅に省メモリ |
| 端末エミュレーション | alacritty_terminal 0.26 | 実績ある VT パーサとグリッド。ConPTY 制御 (`tty`) も同梱 |
| SSH | russh 0.63 (`ring`) | Pure Rust。aws-lc は NASM が必要なため使わない |
| SFTP | russh-sftp 3 | russh のチャネル上で動作 |
| 非同期 | tokio (2 ワーカー) | SSH / SFTP の I/O 専用 |
| ダイアログ | rfd | OS 標準のファイル選択 |
| クリップボード | arboard | 右クリック貼り付け (egui のイベントを介さない読み取り)。Linux は X11 と Wayland |
| インストーラ | Inno Setup 6 | Windows の単一 exe 配布。Linux は tar.gz |

## 2. レイヤー構成

```
app (オーケストレーション: タブ、ダイアログ、ショートカット)
 |-- ui        プレゼンテーション層 (egui 描画と入力)
 |-- session   アプリケーション層 (1 タブ = 端末 + バックエンド)
 |-- backend   I/O 層 (local / ssh / sftp)
 |-- terminal  コア層 (端末状態と UI 非依存の共有状態)
 |-- sshconfig 独立モジュール (OpenSSH config の解決と編集)
 `-- settings  独立モジュール (設定の永続化)
```

依存は上から下への一方向とする。

- `terminal` は egui の `Context` (再描画要求) 以外の UI 要素に依存しない。
- `backend` は `ui` を参照しない。ユーザーへの問い合わせ (パスワード、ホスト鍵確認) は `terminal::Shared::ask` でキューに積み、`ui::prompt_dialog` が取り出して応答する。
- `sshconfig` と `settings` は他モジュールに依存しない。

## 3. モジュール

| パス | 役割 |
| --- | --- |
| `main.rs` | ウィンドウ設定と起動 |
| `app.rs` | 全体状態、タブ管理、バナー、ドロップ処理、ショートカット |
| `settings.rs` | `Settings` の読み書き (`key=value`) |
| `session.rs` | `Session`: `Term` と `Shared` の生成、ローカル / SSH の起動、リサイズ、終了処理 |
| `terminal/shared.rs` | `Shared` (タイトル、状態、サイズ、`PtyIo`、問い合わせキュー)、`Listener` (端末イベント) |
| `terminal/palette.rs` | 配色 (Tokyo Night) と色解決 |
| `backend/mod.rs` | 共有 tokio ランタイム |
| `backend/local.rs` | シェル検出、PTY 起動 (Windows は ConPTY、Linux は POSIX PTY。alacritty の `tty` + `EventLoop`) |
| `backend/ssh/mod.rs` | 接続 (ProxyJump の連鎖)、シェルチャネルの入出力ループ |
| `backend/ssh/handler.rs` | ホスト鍵検証 (known_hosts) |
| `backend/ssh/auth.rs` | 認証の順序制御 |
| `backend/sftp.rs` | `SftpClient`: 一覧、操作、転送 (再帰・進捗・中止) |
| `sshconfig/resolve.rs` | `SshConfig`: Include / Host / Match all の解決、ホスト一覧 |
| `sshconfig/document.rs` | `Document`: 書式を保持した編集用モデル |
| `ui/theme.rs` | 色定数と egui スタイル |
| `ui/fonts.rs` | システムフォントの mmap 読み込み、`TermFont` (セル寸法) |
| `ui/widgets.rs` | 共通ウィジェット |
| `ui/tabbar.rs`, `ui/sidebar.rs` | タブバー、サイドバー。操作は `TabAction` / `SidebarAction` で `app` に返す |
| `ui/terminal/view.rs` | 端末ウィジェット: 入力、選択、スクロール、IME |
| `ui/terminal/render.rs` | グリッド描画 (背景、文字、装飾、カーソル、変換中文字列) |
| `ui/terminal/input.rs` | キー / 貼り付け / マウスのエスケープシーケンス生成 |
| `ui/sftp_panel.rs` | SFTP パネル |
| `ui/config_editor.rs` | SSH config 編集ウィンドウ |
| `ui/prompt_dialog.rs`, `ui/settings_dialog.rs` | 問い合わせダイアログ、設定ダイアログ |

UI コンポーネントは状態を直接変更せず、結果 (`*Action` / `*Result`) を返して `app` が処理する。

## 4. データフロー

### 4.1 端末出力

- ローカル: PTY (Windows は ConPTY、Linux は POSIX PTY) の出力を alacritty の `EventLoop` スレッドがパースして `Term` に反映する。完了したら `Listener` が `Wakeup` で再描画を要求する。
- SSH: tokio タスクがチャネルのデータを `vte::ansi::Processor` に渡して `Term` に反映し、再描画を要求する。同期更新モード (DEC 2026) はタイムアウトで解除する。
- 描画: UI スレッドは毎フレーム `FairMutex` をロックしてグリッドを描画する。

### 4.2 端末入力

- `ui::terminal::view` がキーを `input::encode_key` でバイト列に変換して `Session::write` に渡す。
- `Session::write` は `Shared` の `PtyIo` に渡す。実体はローカルなら `EventLoopSender`、SSH なら mpsc チャネル。
- リサイズは `Term::resize` と `PtyIo::resize` を同時に行う。SSH では window-change を送る。

### 4.3 SSH 接続シーケンス

1. `SshConfig::resolve` で対象と ProxyJump の各ホストを解決する。
2. 各ホップについて次を行う。
   - TCP 接続 (2 段目以降は前段の `direct-tcpip` チャネル) を `ConnectTimeout` 以内に確立する。
   - ハンドシェイクを行う。ホスト鍵の確認はユーザー応答を待つため、タイムアウトの対象外とする。
   - 認証する。
3. PTY を要求してシェルを起動し、入出力ループに入る。
4. 確立した接続は `Link` (`OnceLock`) で公開し、SFTP が同じ接続上に別チャネルを開く。

### 4.4 SFTP

- 操作はすべて tokio タスクで実行し、結果を `Arc<Mutex<Listing>>` と `Transfer` (atomic で進捗を持つ) に書く。UI は毎フレームそれを読む。
- 転送は 256KB 単位でコピーし、チャンクごとに中止フラグを確認する。

## 5. 主要な設計判断

- **省メモリ**
  - レンダラは Glow を使う。eframe の最小アプリでも wgpu は Glow の約 2.5 倍のプライベートメモリを消費する。
  - Glow 自体のベースライン (GPU ドライバ分) は残るが、アプリ自身による増分は数 MB に抑える。
  - フォントは `memmap2` でマップして `'static` として egui に渡し、複製しない。
  - CJK フォールバックには TTC を使う。
  - スクロールバックは alacritty のリングバッファを使う。
- **描画の効率化**: ASCII が連続するセルは 1 回のテキスト描画にまとめる。背景は予約したシェイプスロットに入れて文字の下に描く。
- **UI フォント**: 欧文と和文のベースラインのずれを避けるため、Windows では和文 UI フォント (Yu Gothic UI、なければ Meiryo UI) を主に使う。Linux では Noto Sans CJK などが見つかればそれを使い、無ければ Noto Sans / DejaVu Sans に落とす。
- **config 編集の往復保持**: `Document` は行単位でオプションと生テキスト (コメント・空行) を保持する。変更した項目以外はそのまま書き戻す。
- **Match**: `Match all` 以外の条件は評価できないため、一致しないものとして扱う。誤って設定が適用されるのを防ぐ。
- **ホスト鍵**: 変更された鍵は常に拒否する。未登録の鍵はユーザーが承認した場合のみ登録する。

## 6. ビルドと配布

バージョンの正は `Cargo.toml` の `version` である。`scripts/version.sh` と `scripts/build-installer.ps1` はここから読む。Windows リソースの `FILEVERSION` はビルド時の `CARGO_PKG_VERSION` から生成する。

- `build.rs` は Windows 向けビルドで、`assets/icon.ico` とパッケージバージョンからリソーススクリプトを生成して埋め込む。ウィンドウアイコンは `assets/icon-64.rgba`。
- release プロファイルの設定: `opt-level = "s"`、LTO、`codegen-units = 1`、strip。
- `scripts/build-installer.ps1` の処理:
  1. `cargo build --release` を実行する。
  2. `Cargo.toml` のバージョンを `ISCC /DAppVersion` に渡して `installer/ikterminal.iss` をビルドする。
  3. `target/installer/` に出力する。
- `scripts/package-linux.sh` は release バイナリを `target/dist/IkTerminal-<version>-linux-<arch>.tar.gz` にまとめる。実行時は libxcb、libxkbcommon、libxkbcommon-x11、OpenGL を動的に読む。
- GitHub Actions
  - `Build` は push と pull request で `.github/workflows/package.yml` を呼ぶ。Windows と Linux で clippy、テスト、配布物の生成を行う。
  - `Release` は `main` への push で動く。`version` が push 前のコミットと異なり、`v<version>` タグが無いとき、同じ package ワークフローの成果物を GitHub Release に載せる。失敗した実行の再実行と、手動実行は、タグが無いときに Release を作る。
