# ime-cursor-indicator

マウスカーソル付近に IME の入力モードを表示するツール。

仕様は [SPEC.md](SPEC.md)、設計メモは [DESIGN.md](DESIGN.md) を参照。

## Rust 版

メイン実装は `rust/` です。Python 版は当面併存します。

### 依存パッケージ

Ubuntu の例:

```bash
sudo apt install -y libgtk-3-dev libglib2.0-dev libcairo2-dev libpango1.0-dev
```

トップバー表示（System Tray）を有効にする場合は、実行時ライブラリも入れてください:

```bash
sudo apt install -y libayatana-appindicator3-1
```

### ビルドと実行

```bash
cd rust
cargo run
```

オプション例:

```bash
cargo run -- --poll-ms 60 --opacity 0.8 --offset-x 24 --offset-y 16
```

設定ファイルの優先順位は Python 版と同じです:

`[on]`/`[off]` セクション > コマンドライン引数 / 設定ファイル（トップレベル） > ハードコードデフォルト

### インストール（Ubuntu/X11）

インストールスクリプトを使うと、ビルド・バイナリ配置・設定ファイル・自動起動の設定をまとめて行えます。

```bash
./install.sh
```

install.sh は以下を実行します:

1. `rust/` で `cargo build --release` を実行
2. バイナリを `~/.local/bin/ime-cursor-indicator` に配置
3. `~/.config/ime-cursor-indicator/config.toml` がなければデフォルト設定をコピー（既存の設定は保持）
4. `~/.config/autostart/ime-cursor-indicator.desktop` を生成（ログイン時の自動起動用）

#### 手動インストール

install.sh を使わず手動で行う場合:

```bash
cd rust
cargo build --release
mkdir -p ~/.local/bin
install -m 755 target/release/ime-cursor-indicator ~/.local/bin/ime-cursor-indicator
```

設定ファイル:

```bash
mkdir -p ~/.config/ime-cursor-indicator
cp config.toml.example ~/.config/ime-cursor-indicator/config.toml
```

自動起動設定（`<username>` は実際のユーザー名に置き換え）:

```bash
mkdir -p ~/.config/autostart
cat > ~/.config/autostart/ime-cursor-indicator.desktop <<EOF
[Desktop Entry]
Type=Application
Name=IME Cursor Indicator
Exec=/home/<username>/.local/bin/ime-cursor-indicator
Hidden=false
X-GNOME-Autostart-enabled=true
X-GNOME-Autostart-Delay=4
OnlyShowIn=GNOME;Unity;XFCE;MATE;Cinnamon;
Terminal=false
EOF
```

## 注意

- Mozc のモード判定は IBus プロパティ構造の差異を吸収するため、best-effort の解析です。
- Wayland では動作しません（X11 前提）。
- Chrome では、ページ内テキスト入力要素へフォーカスが移った直後にオーバーレイが追随しない場合があります。Firefox では同現象は再現しにくく、Chrome 側の通知タイミング差が疑われます。
