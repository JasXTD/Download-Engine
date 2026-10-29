# Download Engine

## はじめに

### 起動時に更新を確認

初期設定ではアプリを開くたびに一度確認します。新しいバージョンがあると更新を入手するか後で行うかを選べます。オフラインのエラーは表示せず、環境設定で無効にできます。 公式 GitHub リリースページから更新をダウンロードしてインストールしてください。

環境設定はカテゴリー別のポップアップで開きます。保存で変更を適用し、キャンセル、×、Escape で未保存の編集を破棄します。テーマはプレビューでき、リセットは現在のカテゴリーだけに適用されます。詳細エディターは個別に保存し、閉じると環境設定に戻ります。

### はじめに

ツールバーの「URL を追加」を選び、ファイルの直接リンクを貼り付け、名前・キュー・保存先を確認して開始します。「一括追加」は 1 行に 1 リンクです。「パターンから」を選ぶと、* を 1 つ含むリンクから img001…img100 のような連番リンクを作れます。設定 → 表示 → 言語で変更できます。システムの言語は Windows に従い、未対応なら英語になります。



<!-- custom-themes-3.4.0 -->
### テーマコレクション

環境設定 → 外観 → カスタムで Neon Courier、Reactor Night、Aurora、Moonveilを選べます。背景の強さやアクセントを調整し、PNG/JPEG も取り込めます。保存で適用し、キャンセルで元に戻します。画像はサイズ制限付きでバックグラウンド処理され、ローカルに保存されます。プロファイルのバージョン保護と MCP アクセス制御も含まれます。

PNG / JPEG · 16 MiB、1600万画素まで。このプロファイルにコピーします。
<!-- /custom-themes-3.4.0 -->

## ダウンロードと復旧

### ダウンロードと復旧

ダウンロードをダブルクリックすると進行状況、速度、残り時間を表示します。一時停止と停止は保存済みデータを保持します。「今すぐ開始」は選択したファイルを優先します。再開にはサーバーの対応と同一ファイルの確認が必要です。期限切れリンクは右クリックのリンク更新で復旧します。完了項目の消去は履歴だけを削除し、ファイルは残します。タスク → リストをエクスポート… でダウンロードをファイルに保存し、バックアップや別の PC への移行に使えます。タスク → リストをインポート… は追加前に新しい項目を表示します。既にあるリンクはスキップされ、新しいダウンロードは一時停止のままです。表示 → フローティングドロップボックス で、常に手前に表示される小さなボックスが現れます。Download Engine がトレイにあるときも、リンクをドロップしてダウンロードできます。

## オフラインサイト

### オフラインサイト

「サイト収集」でファイルを探すか、オフラインサイトを保存できます。URL、深度、ページ数、サイズ上限を指定します。結果を確認して追加してください。保存した index.html を開くと閲覧できます。動的処理、ログイン、フォーム、暗号化動画は保存できません。欠落項目はレポートで確認します。

## ブラウザー連携

### ブラウザー連携

ツール → ブラウザー設定で Chrome、Edge、Brave、Firefox を接続します。Chrome、Edge、Brave には実際の拡張機能 ID が必要です。拡張機能の接続確認を使ってください。ブラウザーの設定内で表示言語を選べます。動画ボタンは画質の確認を開きます。サイトログインは明示的な許可後、このファイルだけに使われます。

## キューとスケジュール

### キューとスケジュール

キュー管理でファイルをまとめ、同時ダウンロード数を設定できます。新規キューは開始または予約するまで待機します。スケジューラーで開始、停止、繰り返しを設定します。アプリはトレイを含め起動している必要があります。終了・シャットダウン・スリープ・休止状態は別途有効化し、カウントダウンを取り消せます。

## スクリプト

エディターと読み込み済みスクリプトを別々のタブに分けました。スクリプト一覧で検索と新規作成ができ、Lua または PowerShell エディターのヘッダーから全画面編集を開けます。保存、実行・停止、任意のログ表示をコードとまとめ、ファイル操作をエディター操作メニューに移しました。表示を切り替えても未保存のコード、設定、カーソル位置を保持します。


### スクリプト

ツール → スクリプトでサイトや拡張子に応じたルールを作成します。スクリプトでは Lua または PowerShell を選び、保存して実行します。トリガーは手動、起動時、一定間隔、ダウンロード完了、キュー完了です。実行は重複しません。停止はスクリプトを止めますが、すでに送ったダウンロード指示は残ります。ログで結果を確認してください。

## Lua スクリプト

### Lua スクリプト

Lua は内蔵です。新規スクリプトで Lua と手動トリガーを選び、次の例を保存して実行します。xd.queues() はキュー、xd.downloads() はダウンロードを返します。完了トリガーは event.downloads、キュー完了は event.queue を使います。コマンド名とフィールド名は翻訳しません。詳細は docs/automation.md を参照してください。ログに秘密情報を出力しないでください。PowerShell は Windows アカウントの権限で動作します。

Lua と PowerShell のエディターは、入力が少し止まると保存や実行をせずに構文を自動確認します。行番号、構文の色分け、クリックで移動できるエラーを追加しました。名前の先頭を入力するか Ctrl+Space で候補を表示し、上下キーで選び、Tab、Enter またはクリックで挿入、Esc で閉じます。元に戻す操作に対応し、実行時エラーは引き続き実行ログに表示されます。

```lua
print("Hello from Lua!")
print("Trigger:", event.trigger)
for _, queue in ipairs(xd.queues()) do
    print(queue.name, "running:", queue.running)
end
for _, file in ipairs(event.downloads) do
    print("Finished:", file.name or file.id)
end
```

トリガー「ダウンロード失敗」は、Download Engine 自身の自動再試行の後もダウンロードに確認が必要な場合にスクリプトを実行します。このとき event.downloads には各ダウンロードの error が入ります。xd.retry(id) と xd.restart(id) で再試行し（スクリプトからはダウンロードごとに 5 回まで）、xd.notify(title, text) で Windows 通知を表示し、xd.store.get と xd.store.set で実行間に値を保持できます。xd.rename、xd.move、xd.remove は「ダウンロード済みファイルを変更できる」がオンのときだけファイルを変更します。「サンプルで実行…」は選んだダウンロードでイベントスクリプトを試します。変更はシミュレーションのみです。

```lua
for _, file in ipairs(event.downloads) do
    if pcall(xd.retry, file.id) then
        print("Retrying:", file.name or file.id)
    else
        xd.notify("Download still failing", (file.name or file.id) .. ": " .. (file.error or "no details"))
    end
end
local handled = (xd.store.get("handled") or 0) + #event.downloads
xd.store.set("handled", handled)
print("Failed downloads handled:", handled)
```

別のプログラムでスクリプトを編集するには、エディターの見出しにある「ファイルを開く…」と「ファイルに保存…」を使います。「ファイルを開く…」は .lua または .ps1 ファイルをエディターにコピーし、未保存のコードを置き換える場合は先に確認します。残すには「スクリプトを保存」を選びます。「ファイルに保存…」はエディターのコードをファイルに書き出します。

### Lua Core API 1

Loaded scripts でスクリプトを作成または開き、Script settings で Lua と Manual only を選びます。Full editor で編集領域を広げ、Save script の後に Run script を押し、Run logs を確認します。Core API は現在のトリガーで使える関数、Ctrl+Space は補完を表示します。API 1 はアプリのバージョンとは別の契約番号です。以下はメモリ内の HTML だけを解析し、通信やダウンロードを行いません。手動スクリプトのページ取得対応は false です。

```lua
-- Example: core-api
assert(xd.api_version == 1, "This example needs Lua API 1")
print("App:", xd.app_version, "Role:", xd.script_kind)
local html = '<a class="download" href="/files/demo.zip">Demo</a>'
local links = xd.html_select(html, "a.download[href]", "href")
local url = xd.url_join("https://example.com/catalog/", links[1])
print("Preview:", xd.to_json({ kind = "file", url = url }))
print("Can fetch a page:", xd.supports("xd.fetch"))
```

Open examples/core-api-automation.lua or examples/custom-site-reader.lua with Editor actions → Open file…. Site reader uses public pages, Pages patterns and Direct connection; test with Test on URL before enabling its trigger. Ordinary automation cannot call xd.fetch. Import script… accepts .de-script.json as a new draft with automatic triggers and file changes off; review its code and reselect local queues. Stop script does not undo submitted downloads. Full English reference: docs/automation.md.


### Lua Provider API 1

Lua ダウンロードプロバイダーで取得元の解決、バイト処理、ハッシュ検証と通常の一時停止・再開ができます。各ダウンロードは保存時のスクリプト版を保持します。検証はローカル生成の暗号化ファイルのみです。プロバイダー保存後は旧版アプリがこのプロファイルに書き込めなくなります。

Scripting → Script settings → Download provider → Save script → Add provider download.

API: `resolve(request)`, `refresh(request, previous)`, `transform(plan, bytes, offset)`, `finish(plan, result)`.

`docs/lua-providers.md` · `examples/download-provider.lua` · `examples/download-provider-custom-ctr.lua`

## コマンドライン

### コマンドライン

アプリフォルダーで PowerShell を開き、以下のコマンドを使います。操作指示を受けるにはアプリが起動している必要があります。JSON とコマンド名は英語のままです。空白を含むパスは引用符で囲んでください。

```powershell
.\de-cli.exe --help
.\de-cli.exe queues
.\de-cli.exe list
```

## MCP 連携

### MCP 連携

MCP ページでローカル HTTP サーバーを開始し、クライアント設定をコピーします。最初は閲覧のみを選んでください。AI クライアントには URL と秘密の認証ヘッダーが必要です。トークンを公開しないでください。サーバー停止またはトークン変更でアクセスを取り消せます。詳細は docs/mcp-ai-guide.md にあります。

ツールバー、ツールメニュー、環境設定から MCP をポップアップで開けるようになりました。現在のページは変わりません。× または Escape で閉じても、サーバーは動作を続け、未保存の MCP 編集も保持されます。AI クライアントを切断するにはサーバーを停止してください。環境設定から開いた場合は、閉じると同じ編集内容に戻ります。

## ウイルススキャン

スキャナー: Microsoft Defender, Malwarebytes, Kaspersky, Bitdefender, ClamAV.

英語版の ウイルス対策ソフト エクスプローラー連携が必要です。制限時間は受け渡しのみです。検査の停止は ウイルス対策ソフト で行います。

標準のインストール先が必要です。

保護と隔離はウイルス対策ソフトが管理します。要求の送信は安全な結果を意味しません。

設定を確認: 連携機能が見つかりました。スキャン時に利用可否を確認します。

### ウイルススキャン

ツール → ウイルススキャンを開きます。「単一」は完了済みファイル1つ、「一括」は複数を選択できます。単一スキャンもキャンセルできます。スキャン中や Malwarebytes / Kaspersky / Bitdefender の確認待ちではタブを切り替えられません。

設定でスキャナーを選び、ツール → ウイルススキャン → 一括を開きます。完了ファイルを選択して開始し、結果を確認します。停止は未送信の要求を取り消します。Malwarebytes / Kaspersky / Bitdefender は各要求の後に利用者の確認を待ちます。送信済みスキャンは Malwarebytes / Kaspersky / Bitdefender 側で停止してください。要求済みという表示は安全判定ではありません。

## 設定とログ

### 設定とログ

設定で言語、テーマ、保存先、速度、データ上限、プロキシ、接続数を変更します。ログ画面では検索、絞り込み、コピー、フォルダー表示、診断情報の出力が可能です。実際の URL、パス、スクリプト出力、技術的なエラーは元の文字列を保持します。共有前に診断レポートを確認してください。

インストール版は起動するたびに、別のドライブへ移動した場合も含め、Windows の現在のダウンロードフォルダーを確認します。新しいダウンロードはその場所を使用します。既存のダウンロード、保存済みの進行状況、指定済みのフォルダーは変わりません。ポータブル版は毎回アプリの場所を基準に既定のフォルダーを決めます。旧プロファイルは IDM EX という名前を保持します。場所を取得できない場合はエラーを表示します。

## トラブルシューティング

### トラブルシューティング

接続に失敗したらアプリの起動、拡張機能 ID、登録を確認します。更新後は拡張機能とページを再読み込みしてください。ダウンロード失敗時は空き容量、フォルダー権限、プロキシ、リンク期限を確認します。再開前に復旧データを消さないでください。文字が四角になる場合は Windows の言語フォントを追加し、アプリを再起動します。
