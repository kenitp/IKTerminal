# IkTerminal

Rust 製の軽量タブ型ターミナル (Windows / Linux)。

- ローカルシェル (PowerShell / cmd / WSL)
- シリアル (COM)
- OpenSSH config を読み込む SSH 接続 (ProxyJump、エージェント、鍵、パスワード)。設定で、未起動の Bitwarden を SSH 開始時に起動できる
- SSH 接続上の SFTP ファイル転送
- OpenSSH config の編集 (フォーム / テキスト)
- 設定で、閉じたあとタスクトレイに常駐できる
- リリースビルドは GitHub の最新リリースを確認する。`Ver.Up` で更新し、設定からすぐ確認できる
- SSH セッションで `cursor .` とすると、ローカルの Cursor が Remote-SSH でそのフォルダを開く
- インストール後、エクスプローラーのアドレスバーで `ikt .` とすると、そのフォルダのタブを開く。起動中なら既存のウィンドウに追加する

## ビルド

バージョンは `Cargo.toml` の `version`。

```sh
cargo build --release
```

Windows インストーラ (Inno Setup 6):

```powershell
.\scripts\build-installer.ps1  # target\installer\IkTerminal-<version>-setup.exe
```

Linux アーカイブ:

```sh
./scripts/package-linux.sh  # target/dist/IkTerminal-<version>-linux-<arch>.tar.gz
```

Linux の実行には libxcb、libxkbcommon、libxkbcommon-x11、OpenGL が必要。

## ドキュメント

- [機能仕様](docs/00_spec/functional_spec.md)
- [アーキテクチャ設計](docs/01_design/architecture.md)
