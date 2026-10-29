# Download Engine 使用指南

## 入門

### 啟動時檢查更新

預設在每次開啟程式時檢查一次。發現新版本後可選擇取得更新或稍後處理。離線時不會彈出錯誤，可在偏好設定中關閉此選項。 請從官方 GitHub 發行頁面下載並安裝更新。

偏好設定現在以分類彈出視窗開啟。儲存後套用變更；取消、× 或 Escape 放棄未儲存的編輯。佈景主題可預覽，重設僅影響目前類別。進階編輯器獨立儲存，關閉後返回偏好設定。

### 新增第一個下載

按工具列的「新增 URL」，貼上 HTTP、HTTPS、FTP 或 FTPS 檔案連結，檢查檔名、佇列和儲存位置，再開始下載。「批次新增」支援多行連結。選擇「依模式產生」，可用含一個 * 的連結產生 img001…img100 這類編號連結。預設佇列有可用名額時會自動開始；暫停或排程佇列遵循自己的設定。

在「偏好設定 → 外觀 → 語言」選擇繁體中文、简体中文、English、Bahasa Melayu、日本語、한국어、Tiếng Việt、ไทย、Polski、Русский、Українська、العربية、Deutsch、Français、Svenska、Norsk bokmål、Deutsch (Schweiz)、Italiano 或 Rumantsch Grischun。「系統語言」會跟隨 Windows；不支援的語言使用英語。儲存後套用語言變更，不會變更下載檔案、名稱或指令碼。



<!-- custom-themes-3.4.0 -->
### 主題集

偏好設定 → 外觀 → 自訂提供 Neon Courier、Reactor Night、Aurora 和 Moonveil 主題。可調整背景強度與強調色，或匯入 PNG/JPEG。儲存以保留預覽，取消以還原原外觀。圖片僅存於本機，匯入有大小限制並在背景執行。本版也包含設定檔版本保護和 MCP 權限控制。

PNG / JPEG · 最大 16 MiB、1600 萬像素。複製到目前設定檔。
<!-- /custom-themes-3.4.0 -->

## 下載與復原

### 查看進度與控制下載

按兩下下載或使用右鍵選單開啟下載狀態視窗，查看進度、速度、剩餘時間與連線。暫停保留進度；停止保留可復原資料，並阻止佇列自動重新啟動該工作。繼續可恢復伺服器支援的下載；「立即開始」可直接啟動選取的工作。全部停止影響所有目前工作。工作 → 匯出清單… 可將下載儲存成檔案，以便備份或移到另一台電腦。工作 → 匯入清單… 會先顯示哪些是新項目再新增；已在清單中的連結會被略過，新下載保持暫停。檢視 → 懸浮拖放框 會顯示一個始終置頂的小方框；即使 Download Engine 在系統匣中，也能將連結拖放到上面下載。

復原需要伺服器支援範圍請求且檔案未變更。若連結過期，在右鍵選單選擇復原連結，依瀏覽器提示重新找到同一檔案。確認恢復原本工作，以保留進度並避免重複下載。無法驗證檔案一致性時，應用程式不會盲目合併舊資料。

「清除已完成」只移除清單紀錄，不刪除下載檔案。移除操作會明確詢問是否刪除檔案。

## 離線網站

### 使用擷取工具

工具列中的擷取工具可尋找可下載檔案，也可儲存離線網站。輸入公開網址，限制範圍、深度與數量，先預覽結果，再選擇要加入佇列的檔案。

離線網站功能下載支援的 HTML 頁面與資源，並改寫內部連結。完成後從專案的本機入口開啟。需要伺服器、登入或 JavaScript 動態產生內容的網站可能無法完整離線運作；查看報告中失敗或略過的項目。只擷取你有權儲存的內容。

## 瀏覽器整合

### 連接 Chrome、Edge、Brave 或 Firefox

開啟瀏覽器設定，依各瀏覽器步驟載入擴充功能並註冊連線。Chrome、Edge 和 Brave 需要實際安裝的擴充功能 ID。Firefox 依專用安裝步驟操作。使用擴充功能的連線檢查確認桌面程式能回應。

擴充功能彈出視窗的瀏覽器選項有獨立語言選擇器，預設跟隨瀏覽器。變更後彈出視窗會重新開啟；已開啟的連結選擇頁可重新整理。視訊按鈕不會變更播放內容。

傳送一般連結可直接新增檔案。收集頁面連結後，檢查檔案、佇列和資料夾再提交。視訊按鈕先開啟品質預覽，選擇品質後確認下載。網站登入權限只在你明確選擇時申請；密碼、Cookie 和實際網址不會被翻譯。

## 佇列與排程

### 控制同時下載數量

佇列可分組下載、限制並行數量和安排時間。在管理佇列建立佇列，手動輸入同時下載數量；輸入值受應用程式上限約束。新建佇列等待你啟動或設定排程。

排程支援開始、停止和重複時間。應用程式必須開啟或留在系統匣才會執行排程。檢查時區和選取的佇列。下載完成後結束程式、關機、睡眠或休眠需要另行啟用；倒數可以取消。

## 指令碼

腳本編輯器與已載入腳本現在使用獨立分頁。在已載入腳本中搜尋或新增腳本；在 Lua 或 PowerShell 編輯器標題列中開啟全螢幕編輯器。儲存、執行或停止及選用日誌集中顯示，檔案操作移至「編輯器操作」。切換檢視會保留未儲存的程式碼、設定及游標位置。


### 規則與指令碼

在「工具 → 指令碼」建立規則，可依新下載的網域或副檔名選擇佇列與速度限制，或在一個佇列完成後啟動另一個。空佇列、失敗或未完成檔案不算完成。規則歷程顯示執行結果。

在指令碼頁建立指令碼，輸入名稱、選擇 Lua 或 PowerShell，儲存後執行。停止指令碼會終止它和子處理程序，但已提交的下載命令仍然有效。執行記錄保留程式輸出；藍色為資訊，紅色為錯誤，淺紅色為警告。

自動觸發包含啟動、固定間隔、下載完成和佇列完成。啟用並儲存後才開始監看，不會重播過去事件。指令碼不會與自己重疊執行。可設定逾時限制；關閉應用程式後不會繼續執行。

## Lua 指令碼

### 執行第一個 Lua 指令碼

開啟「工具 → 指令碼 → 指令碼」，建立指令碼，語言選 Lua，觸發方式選手動。內建 Lua 5.4，無需額外安裝。複製下列唯讀範例，儲存並執行，再查看記錄。

Lua 和 PowerShell 編輯器現在會在輸入短暫停頓後自動檢查語法，不儲存或執行腳本。新增行號、語法著色和可點選的錯誤提示。輸入函式名稱前綴或按 Ctrl+Space 顯示建議；用方向鍵選取，按 Tab、Enter 或點選插入，按 Esc 關閉。支援復原，執行階段錯誤仍顯示在執行記錄中。

```lua
print("Hello from Lua!")
print("Trigger:", event.trigger)
for _, queue in ipairs(xd.queues()) do
    print(queue.name, "running:", queue.running)
end
```

xd.downloads() 傳回下載紀錄，xd.queues() 傳回佇列。命令、JSON 欄位與狀態值保持英語，例如 job.status == "complete"。下載完成觸發器透過 event.downloads 提供檔案；佇列完成使用 event.queue。手動觸發時下載清單為空。

```lua
for _, file in ipairs(event.downloads) do
    print("Finished:", file.name or file.id)
    if file.saved_path then print(file.saved_path) end
end
```

local 定義變數，if / end 表示條件，ipairs 逐一處理清單。編輯器內 Tab 縮排；Escape 離開編輯後可用 Tab 導覽。執行前儲存修改。Lua 只能使用提供的應用程式介面；完整 API、下載範例和限制見隨附的 docs/automation.md 英語參考。不要在記錄輸出密碼或權杖。

PowerShell 是選用語言，可使用 Invoke-DownloadEngine queues 和 $Event。它以目前 Windows 帳戶權限執行，只執行可信任的指令碼。

觸發條件「下載失敗」會在下載經過 Download Engine 本身的自動重試後仍需要處理時執行指令碼，此時 event.downloads 包含每個下載的 error。xd.retry(id) 和 xd.restart(id) 會再次嘗試（指令碼對每個下載最多 5 次），xd.notify(title, text) 顯示 Windows 通知，xd.store.get 和 xd.store.set 可在多次執行之間保存值。只有勾選「可以變更已下載的檔案」後，xd.rename、xd.move 和 xd.remove 才能變更下載的檔案。「以範例執行…」可用你選擇的下載試跑事件指令碼，變更僅為模擬。

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

要在其他程式中編輯指令碼，請使用編輯器標題列中的「開啟檔案…」和「儲存到檔案…」。「開啟檔案…」會將 .lua 或 .ps1 檔案複製到編輯器中；如果會取代未儲存的程式碼，會先詢問你。選擇「儲存指令碼」即可保留。「儲存到檔案…」會將編輯器中的程式碼寫入檔案。

### Lua Core API 1

在 Loaded scripts 建立或開啟腳本；Script settings 選擇 Lua 和 Manual only。Full editor 擴大編輯區，Save script 後再 Run script，並在 Run logs 查看結果。Core API 可搜尋目前觸發器支援的函式，Ctrl+Space 提供補全。API 1 是介面版本，與應用程式版本不同。以下範例只解析記憶體中的 HTML，不連線或下載；手動執行時網頁讀取能力為 false。

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

Editor actions → Open file… 可開啟 examples/core-api-automation.lua 或 examples/custom-site-reader.lua。Site reader 使用公開網站、Pages 規則及 Direct connection；先 Test on URL，再啟用自動觸發。一般腳本不能呼叫 xd.fetch。Import script… 讀取 .de-script.json，並關閉自動觸發及檔案修改權限；檢查程式碼、重新選擇佇列後儲存。Stop script 停止腳本，不撤銷已提交的下載。完整步驟見隨附 docs/automation.md。


### Lua Provider API 1

新增 Lua 下載提供者：解析檔案、處理位元組、驗證雜湊，並使用一般下載佇列暫停與繼續。每個工作保留已儲存的指令碼版本。範例僅以本機產生的加密檔案驗證。儲存提供者後，舊版應用程式無法再寫入此設定檔。

Scripting → Script settings → Download provider → Save script → Add provider download.

API: `resolve(request)`, `refresh(request, previous)`, `transform(plan, bytes, offset)`, `finish(plan, result)`.

`docs/lua-providers.md` · `examples/download-provider.lua` · `examples/download-provider-custom-ctr.lua`

## 命令列

### 在 PowerShell 使用 CLI

在應用程式資料夾執行下列唯讀命令。應用程式需要執行才能處理控制命令。命令名稱與 JSON 輸出維持穩定英語格式，方便指令碼使用。

```powershell
.\de-cli.exe --help
.\de-cli.exe queues
.\de-cli.exe list
```

使用 --help 查看新增、暫停、繼續與佇列控制的準確參數。包含空格的路徑需加引號。DOWNLOAD_ENGINE_HOME 可用於隔離測試設定；測試後還原環境變數。

## MCP 整合

### 連接本機 AI 用戶端

開啟 MCP 彈出視窗，啟用本機 HTTP 服務並複製顯示的設定。服務只繫結本機，用戶端需要授權權杖。先使用唯讀模式，確認運作後再開放寫入權限。

權杖授權存取應用程式，請勿公開。停止服務或輪替權杖可撤銷存取。連接埠衝突會顯示錯誤，請選可用的連接埠。完整工具說明見 docs/mcp-ai-guide.md，通訊協定名稱與參數保持英語。

現在可從工具列、工具選單或偏好設定開啟 MCP 彈出視窗，目前頁面保持不變。使用 × 或 Escape 關閉視窗，伺服器會繼續執行，尚未儲存的 MCP 編輯也會保留。要中斷 AI 用戶端，請使用「停止伺服器」。從偏好設定開啟時，關閉後會返回原來的草稿。

## 防毒掃描

掃描程式: Microsoft Defender, Malwarebytes, Kaspersky, Bitdefender, ClamAV.

需要英文版 防毒軟體 檔案總管整合。逾時僅限制要求交接，不限制掃描。請在 防毒軟體 中取消掃描。

需要預設安裝資料夾。

防護和隔離由防毒軟體控制。傳送請求不代表檔案安全。

檢查設定: 已找到整合；要求掃描時會檢查可用性。

### 批次掃描已完成檔案

開啟「工具 → 防毒掃描」，選擇「單個」可掃描一個已完成的下載，選擇「批次」可勾選多個下載。單個模式下可取消掃描；掃描或等待檢視 Malwarebytes / Kaspersky / Bitdefender 結果時無法切換分頁。

在設定選擇支援的掃描器，再開啟「工具 → 防毒掃描 → 批次」。選取已完成檔案或全選，開始後查看進度和各檔案結果。停止批次取消剩餘請求，不刪除下載檔案。

Defender 和命令列掃描器會逐個自動掃描。Malwarebytes / Kaspersky / Bitdefender 檔案總管介面無法回報掃描結束時間，因此每次請求後等待你查看結果，再選掃描下一個。掃描器忙碌時，等空閒再重試目前檔案。已交給 Malwarebytes / Kaspersky / Bitdefender 的掃描可能繼續，請在其介面停止。未驗證的交接不代表檔案安全。

## 設定與記錄

### 儲存偏好與查看問題

設定包含語言、佈景主題、緊湊列、側欄、詳細面板、欄位、下載資料夾、速度限制、配額、代理伺服器和網站連線限制。密碼、登入方式與代理認證依原有方式儲存；切換語言不會修改。

記錄頁顯示時間、層級、元件與活動，可搜尋、篩選、複製、開啟記錄資料夾和匯出診斷。原始伺服器錯誤、指令碼輸出與使用者內容保留原文。分享前檢查匯出內容。

安裝版每次啟動都會讀取 Windows 目前的下載資料夾，包括移至其他磁碟機的位置。新下載使用新位置；既有下載、儲存的進度和自訂資料夾保持不變。可攜版每次啟動都依應用程式所在位置解析預設目錄。舊設定保留 IDM EX 子資料夾名稱。無法取得位置時會顯示錯誤。

## 疑難排解

### 從明確的錯誤開始

連線失敗時確認應用程式正在執行、擴充功能 ID 正確且已註冊。更新後重新載入擴充功能並重新整理網頁。下載失敗時檢查磁碟空間、資料夾權限、代理與過期連結；不要先刪除復原資料再嘗試繼續。

中文字顯示方框時，在 Windows 安裝對應語言字型支援，再重新啟動。應用程式使用本機 Microsoft YaHei、JhengHei 或 SimSun，不重新散布字型。沒有翻譯的技術資訊以英語顯示。說明與變更記錄可離線查看；舊版發行說明和詳細開發參考保留英語原文。
