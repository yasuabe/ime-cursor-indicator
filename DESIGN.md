# ime-cursor-indicator

マウスカーソル付近にIMEの入力モード（日本語/英語）を小さく表示するツール。

## 動機
- 入力モードの誤認による誤送信・誤入力を防ぎたい
- 既存のシステムトレイ表示は視線移動が大きい

## 技術方針（プロトタイプ）
- 言語: Python3（将来的にRust/Goへ移行可能な設計）
- 対象環境: Ubuntu / X11
- IME: IBus (mozc-jp)

## アーキテクチャ
- IBusのD-Bus API (`org.freedesktop.IBus`) でエンジン/モード変化を監視
- マウスカーソル位置をXlibでポーリング（50-100ms）
- X11オーバーレイウィンドウで「あ」「A」等を表示
  - `override_redirect` + 入力フォーカスを奪わない
  - 半透明・最前面

## 依存パッケージ（想定）
- python3-xlib
- dbus-python (or pydbus)

## 将来の改善候補
- AT-SPIによるテキストキャレット追従
- Wayland対応
- Rust/Go移行
