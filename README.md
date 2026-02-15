# ime-cursor-indicator

マウスカーソル付近に IME の入力モードを表示するツール。

詳細は [DESIGN.md](DESIGN.md) を参照。

## Prototype (Ubuntu/X11 + IBus)

`ime_cursor_indicator.py` は `DESIGN.md` に沿った最小プロトタイプです。

- IBus の `GlobalEngineChanged` を監視
- Mozc 利用時は `InputContext.UpdateProperty` を監視して入力モード更新
- カーソル位置を 75ms 間隔で追従
- X11 オーバーレイウィンドウで `A` / `あ` を表示

### 依存パッケージ

Ubuntu の例:

```bash
sudo apt install -y gir1.2-ibus-1.0 python3-gi python3-xlib
```

### 実行

```bash
python3 ime_cursor_indicator.py
```

オプション例:

```bash
python3 ime_cursor_indicator.py --poll-ms 60 --opacity 0.8 --offset-x 24 --offset-y 16
```

### 注意

- Mozc のモード判定は IBus プロパティ構造の差異を吸収するため、best-effort の解析です。
- Wayland では動作しません（X11 前提）。
