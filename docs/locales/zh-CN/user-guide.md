# Download Engine 使用指南

## 入门

### 启动时检查更新

默认在每次打开程序时检查一次。发现新版本后可选择获取更新或稍后处理。离线时不会弹出错误，可在偏好设置中关闭此选项。 请从官方 GitHub 发布页面下载并安装更新。

偏好设置现在以分类弹窗打开。保存后应用更改；取消、× 或 Escape 放弃未保存的编辑。主题可预览，重置仅影响当前类别。高级编辑器独立保存，关闭后返回偏好设置。

### 添加第一个下载

点击工具栏的“添加 URL”，粘贴 HTTP、HTTPS、FTP 或 FTPS 文件链接，检查文件名、队列和保存位置，然后开始下载。“批量添加”支持多行链接。选择“按模式生成”，可用含一个 * 的链接生成 img001…img100 这样的编号链接。默认队列在有可用下载名额时自动开始；暂停或按计划运行的队列遵循自己的设置。

在“偏好设置 → 外观 → 语言”中选择简体中文、繁體中文、English、Bahasa Melayu、日本語、한국어、Tiếng Việt、ไทย、Polski、Русский、Українська、العربية、Deutsch、Français、Svenska、Norsk bokmål、Deutsch (Schweiz)、Italiano 或 Rumantsch Grischun。选择“系统语言”可跟随 Windows；不支持的语言使用英语。保存后应用语言更改，不会改变下载文件、名称或脚本。



<!-- custom-themes-3.4.0 -->
### 主题集

偏好设置 → 外观 → 自定义提供 Neon Courier、Reactor Night、Aurora 和 Moonveil 主题。可调整背景强度与强调色，或导入 PNG/JPEG。保存以保留预览，取消以恢复原外观。图片仅存于本地，导入有大小限制并在后台运行。本版还包含配置版本保护和 MCP 权限控制。

PNG / JPEG · 最大 16 MiB、1600 万像素。复制到当前配置。
<!-- /custom-themes-3.4.0 -->

## 下载与恢复

### 查看进度和控制下载

双击下载或使用右键菜单打开下载状态窗口，查看进度、速度、剩余时间和连接情况。暂停保留进度；停止保留可恢复的数据并阻止队列自动重新启动该任务。恢复可继续受服务器支持的下载；“立即开始”允许直接启动选中的任务。停止全部影响所有当前任务。任务 → 导出列表… 可将下载保存到文件，用于备份或迁移到另一台电脑。任务 → 导入列表… 会先显示哪些是新项目再添加；已在列表中的链接会被跳过，新下载保持暂停。视图 → 悬浮拖放框 会显示一个始终置顶的小框；即使 Download Engine 在托盘中，也可以将链接拖放到上面进行下载。

恢复需要服务器支持范围请求且文件未改变。若链接过期，在右键菜单选择恢复链接并按浏览器提示操作。用浏览器重新找到同一个文件；确认恢复的是原来的任务，以保留进度并避免重复下载。若服务器无法验证文件一致性，应用不会盲目拼接旧数据。

“清除已完成”仅移除完成的列表记录，不删除下载文件。删除操作会明确询问是否删除文件。

## 离线网站

### 使用抓取工具

工具栏中的抓取工具有查找可下载文件和保存离线网站两种用途。输入公开网站地址，限制扫描范围、深度和数量，先预览结果，再选择文件加入队列。

保存离线网站会下载支持的 HTML 页面和资源，并重写内部链接。完成后从项目的本地入口打开。依赖服务器、登录或 JavaScript 动态生成内容的网站可能无法完整离线运行；查看项目报告中的失败或跳过项。只抓取你有权保存的内容。

## 浏览器集成

### 连接 Chrome、Edge、Brave 或 Firefox

打开浏览器设置，按对应浏览器的步骤加载扩展并注册连接。Chrome、Edge 和 Brave 需要填写实际安装的扩展 ID。Firefox 按其专用安装步骤操作。用扩展中的连接检查确认桌面程序能够响应。

扩展弹窗的浏览器选项中有独立的语言选择器；默认跟随浏览器语言。更改后弹窗重新打开，已打开的链接选择页可刷新。视频按钮无需改变播放内容。

发送普通链接可直接添加文件。收集页面链接后，检查文件、队列和保存目录再提交。视频按钮先打开质量预览，选择质量后再确认下载。网站登录权限只在你明确选择时申请，密码、Cookie 和实际网址不会被翻译。

## 队列与计划

### 控制同时下载的数量

队列用于分组下载、限制并发和安排时间。在管理队列中创建队列，手动输入同时下载的数量；输入值受应用上限约束。新建队列等待你启动或设置计划。

计划程序支持开始、停止和重复时间。应用必须保持打开或留在托盘，计划才会执行。检查时区和所选队列。下载完成后的退出、关机、睡眠或休眠操作需要单独启用；倒计时可以取消。

## 脚本

脚本编辑器与已加载脚本现在使用独立标签页。在已加载脚本中搜索或新建脚本；在 Lua 或 PowerShell 编辑器标题栏中打开全屏编辑器。保存、运行或停止及可选日志集中显示，文件操作移至“编辑器操作”。切换视图会保留未保存的代码、设置和光标位置。


### 规则和脚本

在“工具 → 脚本”中创建规则，可根据新下载的域名或扩展名选择队列和速度限制，或在一个队列完成后启动另一个队列。空队列、失败或未完成的文件不算完成。规则历史记录显示运行结果。

在脚本页创建脚本，填写名称，选择 Lua 或 PowerShell，保存后运行。停止脚本会终止脚本及其子进程，但已提交的下载命令仍然有效。运行日志保留程序输出；蓝色为信息，红色为错误，浅红色为警告。

自动触发器包括启动、定时间隔、下载完成和队列完成。启用自动触发并保存后才开始监听；不会重放旧的完成事件。脚本不能与自身重叠运行。设置超时可限制执行时间。关闭应用后不会继续运行。

## Lua 脚本

### 运行第一个 Lua 脚本

打开“工具 → 脚本 → 脚本”，新建脚本，语言选 Lua，触发器选手动。内置 Lua 5.4，无需额外安装。复制以下只读示例，保存后运行，然后查看运行日志。

Lua 和 PowerShell 编辑器现在会在输入短暂停顿后自动检查语法，不保存或执行脚本。新增行号、语法着色和可点击的错误提示。输入函数名前缀或按 Ctrl+Space 显示建议；用方向键选择，按 Tab、Enter 或点击插入，按 Esc 关闭。支持撤销，运行时错误仍显示在运行日志中。

```lua
print("Hello from Lua!")
print("Trigger:", event.trigger)
for _, queue in ipairs(xd.queues()) do
    print(queue.name, "running:", queue.running)
end
```

xd.downloads() 返回下载记录，xd.queues() 返回队列。命令名称、JSON 字段和状态值保持英语，例如 job.status == "complete"。下载完成触发器通过 event.downloads 提供文件；队列完成触发器使用 event.queue。手动触发时下载列表为空。

```lua
for _, file in ipairs(event.downloads) do
    print("Finished:", file.name or file.id)
    if file.saved_path then print(file.saved_path) end
end
```

Lua 使用 local 定义变量，if / end 表示条件，ipairs 遍历列表。编辑器内 Tab 缩进，Escape 退出编辑后可用 Tab 导航。运行前保存修改。Lua 只能使用提供的应用接口；完整 API、下载示例和限制见随包提供的 docs/automation.md 英语参考。不要在日志中输出密码或令牌。

PowerShell 是可选语言，可使用 Invoke-DownloadEngine queues 和 $Event。它以当前 Windows 账户权限执行，仅运行可信脚本。

触发器“下载失败”会在下载经过 Download Engine 自身的自动重试后仍需要处理时运行脚本，此时 event.downloads 包含每个下载的 error。xd.retry(id) 和 xd.restart(id) 会再次尝试（脚本对每个下载最多 5 次），xd.notify(title, text) 显示 Windows 通知，xd.store.get 和 xd.store.set 可在多次运行之间保存值。只有勾选“可以更改已下载的文件”后，xd.rename、xd.move 和 xd.remove 才能更改下载的文件。“用示例运行…”可用你选择的下载试运行事件脚本，更改仅为模拟。

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

要在其他程序中编辑脚本，请使用编辑器标题栏中的“打开文件…”和“保存到文件…”。“打开文件…”会将 .lua 或 .ps1 文件复制到编辑器中；如果会替换未保存的代码，会先询问你。选择“保存脚本”即可保留。“保存到文件…”会将编辑器中的代码写入文件。

### Lua Core API 1

在 Loaded scripts 中创建或打开脚本；Script settings 选择 Lua 和 Manual only。Full editor 扩大编辑区，Save script 后再 Run script，并在 Run logs 查看结果。Core API 可搜索当前触发器支持的函数，Ctrl+Space 提供补全。API 1 是接口版本，与应用版本不同。下面的示例只解析内存中的 HTML，不访问网站或下载文件；手动运行时页面读取能力为 false。

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

Editor actions → Open file… 可打开 examples/core-api-automation.lua 或 examples/custom-site-reader.lua。Site reader 使用公共网站、Pages 匹配规则与 Direct connection；先 Test on URL，再启用自动触发。普通脚本不能调用 xd.fetch。Import script… 读取 .de-script.json，并关闭自动触发及文件修改权限；检查代码、重新选择队列后保存。Stop script 停止脚本，不撤销已提交的下载。完整步骤见随包 docs/automation.md。


### Lua Provider API 1

新增 Lua 下载提供程序：解析文件、处理字节、校验哈希并使用常规下载队列暂停和恢复。每个任务保留已保存的脚本版本。示例仅使用本地生成的加密文件验证。保存提供程序后，旧版应用不能再写入此配置文件。

Scripting → Script settings → Download provider → Save script → Add provider download.

API: `resolve(request)`, `refresh(request, previous)`, `transform(plan, bytes, offset)`, `finish(plan, result)`.

`docs/lua-providers.md` · `examples/download-provider.lua` · `examples/download-provider-custom-ctr.lua`

## 命令行

### 在 PowerShell 中使用 CLI

在应用文件夹中运行下列只读命令。应用需要运行才能处理控制命令。命令名称和 JSON 输出保持稳定的英语格式，便于脚本使用。

```powershell
.\de-cli.exe --help
.\de-cli.exe queues
.\de-cli.exe list
```

通过 --help 查看添加、暂停、恢复和队列控制的准确参数。引号包住包含空格的路径。DOWNLOAD_ENGINE_HOME 可用于隔离测试配置；测试结束后恢复环境变量。

## MCP 集成

### 连接本地 AI 客户端

打开 MCP 弹窗，启用本地 HTTP 服务并复制弹窗显示的配置。服务仅绑定本机；客户端需要授权令牌。默认先使用只读模式，确认客户端工作正常后再开放写入权限。

令牌用于授权访问应用，不要公开。停止服务或轮换令牌可撤销访问。端口冲突会显示错误，请选择可用端口。完整工具说明见 docs/mcp-ai-guide.md，协议名称和参数保持英语。

现在可从工具栏、工具菜单或偏好设置打开 MCP 弹窗，当前页面保持不变。使用 × 或 Escape 关闭弹窗，服务器会继续运行，尚未保存的 MCP 编辑也会保留。要断开 AI 客户端，请使用“停止服务器”。从偏好设置打开时，关闭后会返回原来的草稿。

## 防病毒扫描

扫描程序: Microsoft Defender, Malwarebytes, Kaspersky, Bitdefender, ClamAV.

需要英文版 杀毒软件 资源管理器集成。超时仅限制请求交接，不限制扫描。请在 杀毒软件 中取消扫描。

需要默认安装文件夹。

防护和隔离由杀毒软件控制。发送请求不代表文件安全。

检查设置: 已找到集成；请求扫描时会检查可用性。

### 批量扫描已完成文件

打开“工具 → 杀毒扫描”，选择“单个”可扫描一个已完成的下载，选择“批量”可勾选多个下载。单个模式下可取消扫描；扫描或等待查看 Malwarebytes / Kaspersky / Bitdefender 结果时无法切换标签。

在设置中选择受支持的扫描器，然后打开“工具 → 杀毒扫描 → 批量”。选择已完成的文件或全选，开始后查看进度和每个文件的结果。停止批次会取消剩余请求，不删除下载文件。

Defender 和命令行扫描器逐个自动扫描。Malwarebytes / Kaspersky / Bitdefender 的资源管理器接口不能报告扫描结束时间，因此每个请求后等待你查看结果再选择扫描下一个。若扫描器忙碌，等其空闲后重试当前文件。已交给 Malwarebytes / Kaspersky / Bitdefender 的扫描可能继续；在它自己的界面停止。未验证的交接不能视为文件安全。

## 设置与日志

### 保存偏好和查看问题

设置包含语言、主题、紧凑行、侧栏、详情面板、列、下载文件夹、速度限制、配额、代理和站点连接限制。密码、登录方式和代理凭据按原有方式保存，切换语言不会改动它们。

日志页面显示时间、级别、组件和活动，支持搜索、筛选、复制、打开日志文件夹和导出诊断。原始服务器错误、脚本输出和用户内容保留原文。导出前检查是否包含你不想分享的信息。

安装版每次启动都会读取 Windows 当前的下载文件夹，包括移动到其他驱动器的位置。新下载使用新位置；已有下载、保存的进度和自定义文件夹保持不变。便携版每次启动都按应用所在位置解析默认目录。旧配置保留 IDM EX 子文件夹名称。无法获取位置时会显示错误。

## 故障排除

### 从明确的错误开始

连接失败时先确认应用正在运行、扩展 ID 正确且连接已注册。扩展更新后重新加载扩展并刷新目标网页。下载失败时检查剩余空间、保存目录权限、代理和过期链接；不要删除恢复数据后再尝试恢复。

中文显示为方框时，在 Windows 中安装相应语言的字体支持，然后重启应用。应用使用本机 Microsoft YaHei、JhengHei 或 SimSun 字体，不随程序重新分发字体。找不到翻译的技术信息会显示英语。帮助和更改日志可离线查看；旧版发行说明和详细开发参考保留英语原文。
