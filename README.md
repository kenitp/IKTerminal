# IkTerminal

Rust 製の軽量タブ型ターミナル (Windows 向け)。

- ローカルシェル (PowerShell / cmd / WSL)
- OpenSSH config を読み込む SSH 接続 (ProxyJump、エージェント、鍵、パスワード)
- SSH 接続上の SFTP ファイル転送
- OpenSSH config の編集 (フォーム / テキスト)

## ビルド

```powershell
cargo build --release          # target\release\ikterminal.exe
.\scripts\build-installer.ps1  # target\installer\IkTerminal-<version>-setup.exe (要 Inno Setup 6)
```

## ドキュメント

- [機能仕様](docs/00_spec/functional_spec.md)
- [アーキテクチャ設計](docs/01_design/architecture.md)
