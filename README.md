# ime-cursor-indicator

マウスカーソル付近に IME の入力モードを表示するツール。

仕様は [SPEC.md](SPEC.md)、設計メモは [DESIGN.md](DESIGN.md) を参照。

## Python 版 (Prototype)

`python/ime_cursor_indicator.py` は最小プロトタイプです。現在は Rust への移植を進めています（[MIGRATION.md](MIGRATION.md)）。

- IBus の `GlobalEngineChanged` を監視
- Mozc 利用時は `InputContext.UpdateProperty` を監視して入力モード更新
- カーソル位置を 75ms 間隔で追従
- X11 オーバーレイウィンドウで `A` / `あ` を表示

### 依存パッケージ

Ubuntu の例:

```bash
sudo apt install -y gir1.2-ibus-1.0 python3-gi python3-xlib \
    gir1.2-pango-1.0 gir1.2-pangocairo-1.0 python3-cairo \
    gir1.2-ayatanaappindicator3-0.1
```

### 実行

```bash
python3 python/ime_cursor_indicator.py
```

オプション例:

```bash
python3 python/ime_cursor_indicator.py --poll-ms 60 --opacity 0.8 --offset-x 24 --offset-y 16
```

### 設定ファイル

`~/.config/ime-cursor-indicator/config.toml` に設定を記述すると、デフォルト値を変更できます。

```bash
mkdir -p ~/.config/ime-cursor-indicator
cp config.toml.example ~/.config/ime-cursor-indicator/config.toml
```

設定例:

```toml
poll_ms = 60
opacity = 0.8
offset_x = 24
```

優先順位: 設定ファイル `[on]`/`[off]` セクション > コマンドライン引数 / 設定ファイル（トップレベル） > ハードコードデフォルト

### 自動起動

ログイン時に自動起動するには、`.desktop` ファイルを `~/.config/autostart/` に配置します。

```bash
mkdir -p ~/.config/autostart
cp ime-cursor-indicator.desktop.example ~/.config/autostart/ime-cursor-indicator.desktop
```

`Exec=` の `/path/to/ime_cursor_indicator.py` を実際のパスに書き換えてください。

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

## 注意

- Mozc のモード判定は IBus プロパティ構造の差異を吸収するため、best-effort の解析です。
- Wayland では動作しません（X11 前提）。
