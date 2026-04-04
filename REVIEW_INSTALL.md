# install.sh レビュー方針

## 経緯

install.sh のレビューで `<<'DESKTOP_EOF'`（クォート付きヒアドキュメント）による `$HOME` 未展開を見落とし、生成された .desktop ファイルの `Exec` がリテラル `$HOME/...` のまま書き込まれた。.desktop ファイルは環境変数を解釈しないため、自動起動が壊れた。

この教訓を踏まえ、install.sh のレビュー観点を定める。

## install.sh の処理と各ステップのチェック項目

### ステップ 1: Rust バイナリのビルド (`cargo build --release`)

- cargo / rustc が未インストールの場合、エラーメッセージから原因を特定できるか
- ランタイム依存（libgtk-3-dev, libayatana-appindicator3-1 等）が不足している場合のリンクエラーが分かりやすいか

### ステップ 2: バイナリの配置 (`~/.local/bin/`)

- `$XDG_DATA_HOME` 等の XDG 変数を尊重しているか
- パーミッション（755）が明示されているか
- 既存プロセスが動作中の場合の案内があるか

### ステップ 3: config.toml のコピー (`~/.config/ime-cursor-indicator/`)

- `$XDG_CONFIG_HOME` を尊重しているか（`~/.config` のハードコードでないか）
- config.toml.example の内容と整合しているか
- 既存 config がある場合に上書きしない冪等性が保たれているか

### ステップ 4: .desktop ファイルの生成 (`~/.config/autostart/`)

- **ヒアドキュメントのクォート有無**: `<<'EOF'` だとシェル変数が展開されない。.desktop は環境変数を解釈しないため、`Exec` のパスはインストール時に展開済みの絶対パスでなければならない
- Desktop Entry Specification に準拠しているか（`Exec` キーの書式等）
- `OnlyShowIn` の制限が妥当か（IBus は DE に依存しないため、不要な制限になっていないか）
- パーミッションが umask 依存でなく明示されているか
- 生成後のファイル内容を実際に確認したか（静的なコードリーディングだけで済ませない）

## プロジェクト内の関連ファイルとの整合性

install.sh を変更したら、以下のファイルとの乖離がないか確認する。

- **README.md** — 「実運用」セクションのインストール手順
- **ime-cursor-indicator.desktop.example** — テンプレートとの二重管理になっていないか
- **SPEC.md** — 設定ファイルのパス・パラメータの記載
- **config.toml.example** — コピー元の内容

## 過去に発見された問題

| 発見日 | 重要度 | 内容 |
|--------|--------|------|
| 2026-04-05 | High | `<<'DESKTOP_EOF'` で `$HOME` が未展開、自動起動が壊れた |
| 2026-04-05 | Medium | XDG Base Directory 非準拠（`~/.config` ハードコード） |
| 2026-04-05 | Medium | `OnlyShowIn` が KDE + IBus 等を不要に除外 |
| 2026-04-05 | Medium | .desktop.example との二重管理 |
