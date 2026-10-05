# IkTerminal アーキテクチャ設計

## 1. 技術選定

| 用途 | 採用 | 理由 |
| --- | --- | --- |
| GUI | eframe / egui 0.35 (Glow) | 即時モードで状態管理が単純。wgpu より大幅に省メモリ |
| 端末エミュレーション | alacritty_terminal 0.26 | 実績ある VT パーサとグリッド。ConPTY 制御 (`tty`) も同梱 |
| SSH | russh 0.63 (`ring`) | Pure Rust。aws-lc は NASM が必要なため使わない |
| SFTP | russh-sftp 3 | russh のチャネル上で動作 |
| 非同期 | tokio (2 ワーカー) | SSH / SFTP の I/O 専用 |
| シリアル | serialport 4 (`default-features = false`) | COM ポートの列挙と読み書き。libudev は使わない |
| ダイアログ | rfd | OS 標準のファイル選択 |
| クリップボード | arboard | 右クリック貼り付け (egui のイベントを介さない読み取り)。Linux は X11 と Wayland |
| インストーラ | Inno Setup 6 | Windows の単一 exe 配布。Linux は tar.gz |
| 更新の HTTP | Windows は WinHTTP。Linux は ureq 2.9 (`rustls` + `ring`) | 暗号は ring に揃える。aws-lc は使わない |

## 2. レイヤー構成

```
app (オーケストレーション: タブ、ダイアログ、ショートカット)
 |-- ui        プレゼンテーション層 (egui 描画と入力)
 |-- session   アプリケーション層 (1 タブ = 端末 + バックエンド)
 |-- backend   I/O 層 (local / ssh / sftp / serial)
 |-- terminal  コア層 (端末状態と UI 非依存の共有状態)
 |-- sshconfig 独立モジュール (OpenSSH config の解決と編集)
 |-- settings  独立モジュール (設定の永続化)
 `-- update    独立モジュール (最新 Release の確認と適用)
```

依存は上から下への一方向とする。

- `terminal` は egui の `Context` (再描画要求) 以外の UI 要素に依存しない。
- `backend` は `ui` を参照しない。ユーザーへの問い合わせ (パスワード、ホスト鍵確認) は `terminal::Shared::ask` でキューに積み、`ui::prompt_dialog` が取り出して応答する。
- `sshconfig` と `settings` は他モジュールに依存しない。
- `update` は `app` から使う。`ui` には依存しない。

## 3. モジュール

| パス | 役割 |
| --- | --- |
| `main.rs` | ウィンドウ設定と起動。OS のタイトルバーは出さない |
| `launch.rs` | 起動引数のフォルダ解決 (`ikt .`) |
| `instance.rs` | 単一インスタンス。後続の起動は既存プロセスへフォルダを渡して終了する |
| `update.rs` | 最新 Release の確認と、`Ver.Up` で始める更新。`app` だけが使う |
| `frame.rs` | OS タイトルバーを消す。Windows ではこのスレッドの winit ウィンドウから `WS_CAPTION` を外す |
| `app.rs` | 全体状態、タブと複数ウィンドウ、バナー、ドロップ処理、ショートカット |
| `settings.rs` | `Settings` の読み書き (`key=value`) |
| `session.rs` | `Session`: `Term` と `Shared` の生成、ローカル / SSH / シリアルの起動、リサイズ、終了処理 |
| `terminal/shared.rs` | `Shared` (タイトル、状態、サイズ、`PtyIo`、問い合わせキュー)、`Listener` (端末イベント) |
| `terminal/palette.rs` | 配色 (Tokyo Night) と色解決 |
| `backend/mod.rs` | 共有 tokio ランタイム |
| `backend/local.rs` | シェル検出、PTY 起動 (Windows は ConPTY、Linux は POSIX PTY。alacritty の `tty` + `EventLoop`) |
| `backend/cwd.rs` | ローカルシェルの作業ディレクトリ (Windows はプロセスの PEB、Linux は `/proc/<pid>/cwd`) |
| `backend/serial.rs` | COM ポートの列挙と、8N1 でのバイト転送 |
| `backend/bitwarden.rs` | SSH 前に Bitwarden デスクトップが止まっていれば起動する |
| `backend/ssh/mod.rs` | 接続 (ProxyJump の連鎖)、シェルチャネルの入出力ループ |
| `backend/ssh/handler.rs` | ホスト鍵検証 (known_hosts) |
| `backend/ssh/auth.rs` | 認証の順序制御 |
| `backend/sftp.rs` | `SftpClient`: 一覧、操作、転送 (再帰・進捗・中止) |
| `sshconfig/resolve.rs` | `SshConfig`: Include / Host / Match all の解決、ホスト一覧 |
| `sshconfig/document.rs` | `Document`: 書式を保持した編集用モデル |
| `sshconfig/command.rs` | シェル行から `ssh` コマンドを取り出し、config へ追記する |
| `ui/theme.rs` | 色定数と egui スタイル |
| `ui/fonts.rs` | システムフォントの mmap 読み込み、`TermFont` (セル寸法) |
| `ui/widgets.rs` | 共通ウィジェット |
| `ui/tabbar.rs`, `ui/sidebar.rs` | タブバー (ウィンドウ移動、タブの並べ替えと切り離し、新規タブメニュー、ディレクトリのホバー)、サイドバー。操作は `TabAction` / `SidebarAction` で `app` に返す |
| `ui/chrome.rs` | 枠なしウィンドウのリサイズ端 |
| `ui/ssh_save_dialog.rs` | `ssh` コマンドを config に追加するか尋ねるダイアログ |
| `ui/serial_dialog.rs` | COM ポートとボーレートの選択 |
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
- シリアル: 専用スレッドが COM ポートを読み、同じパーサに渡す。ウィンドウサイズは端末グリッドだけを変える。
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
- **枠なしウィンドウ**: タブバーの空きをドラッグ領域にし、端のドラッグは `BeginResize` で OS に渡す。スナップと最小サイズは OS に任せる。Windows では `WS_CAPTION` を外す。winit がスタイル更新でビットを戻すため、戻っていたら毎フレーム外し直す。タブを切り離したウィンドウも同じ扱いである。
- **タブの切り離し**: セッション (PTY / SSH) はプロセスをまたいで移せない。ドラッグで分けたウィンドウは同じプロセスの egui viewport として描く。子ウィンドウは親の再描画に合わせて描く。
- **単一インスタンス**: 最初のプロセスが待つ。Windows はログオンセッションごとの名前付きパイプ、Linux はランタイムディレクトリの unix socket。後続プロセスはフォルダを 1 行送って終了する。受信側はフォーカス中のウィンドウにローカルタブを足す。
- **更新**: リリースビルドだけが起動後に最新 Release を見る。確認とダウンロードは UI スレッドの外で行う。配布物は GitHub が付ける SHA-256 と照合してから、`Ver.Up` を押したときに適用し、アプリを終了する。Windows の取得は WinHTTP、Linux は rustls である。Windows はサイレントインストーラをプロセス終了後に実行し、Linux は実行中のバイナリを置き換える。
- **起動フォルダ**: 引数があるときだけ作業ディレクトリにする。スタートメニュー起動時のカレントフォルダ (System32 など) は使わない。
- **タブのフォルダ名**: ローカルシェルのプロセスから作業ディレクトリを読む。UI は約 0.5 秒ごとに `session` 経由で取得し、`フォルダ · シェル名` とフルパスのホバーを描く。SSH とシリアルは対象外。WSL の Linux 側ディレクトリは読まない。
- **`ssh` の検出**: Enter の時点でカーソル行 (折り返しを含む) を読み、コマンド位置の `ssh` だけを解釈する。保存は `Document` 経由で、他の行を崩さない。
- **Bitwarden**: 設定が有効なときだけ、SSH 接続の直前にプロセスを確認する。未起動なら、インストーラ版は `Bitwarden.exe`、Microsoft Store 版は `shell:AppsFolder` のアプリ ID で起動し、`openssh-ssh-agent` のパイプを待ってから認証する。
- **シリアル**: ポートの開閉は UI ではなく `backend::serial` が行う。失敗はダイアログに返し、成功したらタブを追加する。

## 6. ビルドと配布

バージョンの正は `Cargo.toml` の `version` である。`scripts/version.sh` と `scripts/build-installer.ps1` はここから読む。Windows リソースの `FILEVERSION` はビルド時の `CARGO_PKG_VERSION` から生成する。

- `build.rs` は Windows 向けビルドで、`assets/icon.ico` とパッケージバージョンからリソーススクリプトを生成して埋め込む。ウィンドウアイコンは `assets/icon-64.rgba`。
- インストーラは `ikterminal.exe` と同一の `ikt.exe` を置き、インストール先をユーザーの PATH と App Paths に登録する。
- release プロファイルの設定: `opt-level = "s"`、LTO、`codegen-units = 1`、strip。
- `scripts/build-installer.ps1` の処理:
  1. `cargo build --release` を実行する。
  2. `Cargo.toml` のバージョンを `ISCC /DAppVersion` に渡して `installer/ikterminal.iss` をビルドする。
  3. `target/installer/` に出力する。
- `scripts/package-linux.sh` は release バイナリを `target/dist/IkTerminal-<version>-linux-<arch>.tar.gz` にまとめる。実行時は libxcb、libxkbcommon、libxkbcommon-x11、OpenGL を動的に読む。
- GitHub Actions
  - `Build` は push と pull request で `.github/workflows/package.yml` を呼ぶ。Windows と Linux で clippy、テスト、配布物の生成を行う。
  - `Release` は `main` への push で動く。`version` が push 前のコミットと異なり、`v<version>` タグが無いとき、同じ package ワークフローの成果物を GitHub Release に載せる。失敗した実行の再実行と、手動実行は、タグが無いときに Release を作る。
