# KNOWLEDGE.md

開発中に得られた教訓・知見を記録する。

## IBus は専用バス (private bus) を使う

- IBus はセッションバス (`dbus.SessionBus()`) ではなく、独自の D-Bus バスで通信する
- Python: `dbus-python` では接続不能。`gi.repository.IBus` に移行して解決 (`c854eab`)
- Rust: `zbus` v4 は IBus 専用バスの非標準 sender 名でパニックする。`gio::DBusConnection` に切り替えて解決 (`d474384`)
- **教訓**: 専用バスを持つサービスに接続する際は、汎用 D-Bus ライブラリが対応しているか事前に確認すること

## X11 コアフォントに CJK は含まれない

- X11 の misc-fixed フォントでは「あ」が文字化けする
- Cairo + Pango に移行してシステムフォント (Unicode 対応) を利用することで解決 (`148fc47`)

## キャレット消失の誤判定

- 時間ベースの stale 検出 (1.2 秒タイムアウト) は、タイプ中にキャレットが静止しているだけで「消失」と誤判定する
- IBus `FocusOut` メソッド呼び出しを eavesdrop することで、正確にフォーカス消失を検出できる (`425cd9d`)
- **教訓**: タイムアウトによるヒューリスティックより、明示的なイベント (FocusOut) に基づく検出を優先すべき

## SetCursorLocation はアプリ側が送る

- IBus の `SetCursorLocation(x, y, w, h)` はアプリケーションから IBus への method call
- IBus 側から現在のキャレット位置を問い合わせる手段はない
- フォーカス移動時にアプリが `SetCursorLocation` を送らない場合、タイプするまでオーバーレイが追従しない (TODO 項目)

## AT-SPI によるキャレット追跡 (調査済み・未実装)

- AT-SPI (Assistive Technology Service Provider Interface) は専用の D-Bus バスを持つ (`org.a11y.Bus` の `GetAddress` で取得)
- `object:text-caret-moved` イベントを購読し、`org.a11y.atspi.Text.GetCharacterExtents(offset, coord_type=0)` でスクリーン座標を取得可能
- GTK, Qt, Firefox, Chromium が対応。カスタムアプリは非対応の場合あり
- Rust: `atspi` クレート (zbus ベース) が存在するが、zbus は IBus 専用バスで問題を起こした前例がある。gio D-Bus 直接利用も選択肢
- 空テキストでは `GetCharacterExtents` が `(-1, -1)` を返す場合がある
- **注意**: AT-SPI も専用バスなので、IBus と同じ接続問題が起こりうる

## AT-SPI フォーカス観測の注意点

- `object:state-changed:focused` 自体は広く発生するが、ブラウザ内 `input` / `textarea` では `window` や `panel` の focused ばかり見え、入力要素自身を直接示さない場合がある
- `focused` 以外にも `PropertyChange`、`TextChanged`、`VisibleDataChanged`、`BoundsChanged` など多数のオブジェクトイベントが出るため、広い購読だけでは目的イベントを特定しにくい
- `Alt+Tab` や `Tab` を境界に後から必要区間を抽出するには、POC のログにタイムスタンプと連番を付け、ファイルへ保存しておくのが有効

## ブラウザ差異: Firefox と Chrome

- Firefox では、ページ内テキスト入力要素へフォーカスが移ると、オーバーレイは概ね期待通りに追随する
- Chrome では、ページ内テキスト入力要素へフォーカスが移った直後はオーバーレイが追随しない場合がある
- ただし Chrome でも、フォーカス後に何らかのキーイベントが入るとオーバーレイがキャレット位置へ移動することがある
- この差から、Chrome ではフォーカス移動直後の `SetCursorLocation` 相当の通知が弱い、または遅延している可能性が高い
