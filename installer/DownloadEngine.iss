#ifndef SourceRoot
  #define SourceRoot ".."
#endif
#ifndef AppVersion
  #error AppVersion is required. Use scripts/Build-Installer.ps1.
#endif
#ifndef ReleaseOutputDir
  #error ReleaseOutputDir is required. Use scripts/Build-Installer.ps1.
#endif
#ifdef TestBuild
  #define AppIdentity "IDM_EX.DownloadManager.SmokeTest"
  #define OutputName "DownloadEngine-" + AppVersion + "-TestSetup"
  ; The smoke test checks startup cleanup in an isolated key, never the real Run key.
  #define StartupRunKey "Software\IDM_EX\Tests\InstallerStartup\Run"
  #define StartupApprovedKey "Software\IDM_EX\Tests\InstallerStartup\StartupApproved"
#else
  #define StartupRunKey "Software\Microsoft\Windows\CurrentVersion\Run"
  #define StartupApprovedKey "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"
  #define AppIdentity "IDM_EX.DownloadManager"
  #ifdef UnsignedRelease
    #define OutputName "DownloadEngine-" + AppVersion + "-UnsignedSetup"
  #else
    #define OutputName "DownloadEngine-" + AppVersion + "-Setup"
  #endif
#endif

[Setup]
AppId={#AppIdentity}
AppName=Download Engine
AppVersion={#AppVersion}
VersionInfoVersion={#AppVersion}
; New installs use the new folder; upgrades keep the folder of the existing installation.
DefaultDirName={localappdata}\Programs\Download Engine
DefaultGroupName=Download Engine
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#ReleaseOutputDir}
OutputBaseFilename={#OutputName}
SetupIconFile={#SourceRoot}\assets\download-engine.ico
UninstallDisplayIcon={app}\de.exe
UninstallDisplayName=Download Engine
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
DisableProgramGroupPage=yes
DisableDirPage=no
AppMutex=Local\IDM_EX.Installed
SetupMutex=Local\IDM_EX.Setup
CloseApplications=no
RestartApplications=no
AllowNoIcons=yes
ChangesAssociations=no
ShowLanguageDialog=yes
UsePreviousLanguage=yes
#ifdef SignRelease
SignTool=idmex
SignedUninstaller=yes
SignToolRetryCount=3
SignToolRetryDelay=2500
#else
SignedUninstaller=no
#endif

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "zh_CN"; MessagesFile: "{#SourceRoot}\installer\languages\ChineseSimplified.isl"
Name: "zh_TW"; MessagesFile: "{#SourceRoot}\installer\languages\ChineseTraditional.isl"
Name: "ms"; MessagesFile: "{#SourceRoot}\installer\languages\Malaysian.isl"

Name: "ja"; MessagesFile: "{#SourceRoot}\installer\languages\Japanese.isl"
Name: "ko"; MessagesFile: "{#SourceRoot}\installer\languages\Korean.isl"
Name: "vi"; MessagesFile: "{#SourceRoot}\installer\languages\Vietnamese.isl"
Name: "th"; MessagesFile: "{#SourceRoot}\installer\languages\Thai.isl"
Name: "pl"; MessagesFile: "{#SourceRoot}\installer\languages\Polish.isl"
Name: "ru"; MessagesFile: "{#SourceRoot}\installer\languages\Russian.isl"
Name: "uk"; MessagesFile: "{#SourceRoot}\installer\languages\Ukrainian.isl"
Name: "ar"; MessagesFile: "{#SourceRoot}\installer\languages\Arabic.isl"

Name: "de"; MessagesFile: "{#SourceRoot}\installer\languages\German.isl"
Name: "fr"; MessagesFile: "{#SourceRoot}\installer\languages\French.isl"
Name: "sv"; MessagesFile: "{#SourceRoot}\installer\languages\Swedish.isl"
Name: "no"; MessagesFile: "{#SourceRoot}\installer\languages\Norwegian.isl"
Name: "de_CH"; MessagesFile: "{#SourceRoot}\installer\languages\GermanSwiss.isl"

Name: "it"; MessagesFile: "{#SourceRoot}\installer\languages\Italian.isl"
Name: "rm"; MessagesFile: "{#SourceRoot}\installer\languages\Romansh.isl"

[LangOptions]
no.LanguageName=Norsk bokmål

[CustomMessages]
en.ConnectBrowser=Open Download Engine and connect a browser
zh_CN.ConnectBrowser=打开 Download Engine 并连接浏览器
zh_TW.ConnectBrowser=開啟 Download Engine 並連接瀏覽器
ms.ConnectBrowser=Buka Download Engine dan sambungkan pelayar
en.PortableFolder=This is a portable Download Engine folder. Choose a different installation folder to preserve its queue and downloads.
zh_CN.PortableFolder=这是 Download Engine 便携版文件夹。请选择其他安装文件夹，以保留队列和下载文件。
zh_TW.PortableFolder=這是 Download Engine 可攜版資料夾。請選擇其他安裝資料夾，以保留佇列和下載檔案。
ms.PortableFolder=Ini ialah folder Download Engine mudah alih. Pilih folder pemasangan lain untuk mengekalkan baris gilir dan muat turunnya.
en.OtherCopy=This folder contains another Download Engine copy. Choose a different installation folder.
zh_CN.OtherCopy=此文件夹包含另一个 Download Engine 副本。请选择其他安装文件夹。
zh_TW.OtherCopy=此資料夾包含另一個 Download Engine 副本。請選擇其他安裝資料夾。
ms.OtherCopy=Folder ini mengandungi salinan Download Engine lain. Pilih folder pemasangan yang berbeza.
en.MissingCleanup=Browser cleanup is missing. Repair this installation before uninstalling.
zh_CN.MissingCleanup=缺少浏览器清理程序。请先修复安装，再卸载。
zh_TW.MissingCleanup=缺少瀏覽器清理程式。請先修復安裝，再解除安裝。
ms.MissingCleanup=Pembersihan pelayar tiada. Baiki pemasangan ini sebelum menyahpasang.
en.CannotCleanup=Cannot start browser cleanup. Repair this installation before uninstalling.
zh_CN.CannotCleanup=无法启动浏览器清理程序。请先修复安装，再卸载。
zh_TW.CannotCleanup=無法啟動瀏覽器清理程式。請先修復安裝，再解除安裝。
ms.CannotCleanup=Tidak dapat memulakan pembersihan pelayar. Baiki pemasangan ini sebelum menyahpasang.
en.CleanupFailed=Browser cleanup failed. Close Browser setup and retry. The application has been kept.
zh_CN.CleanupFailed=浏览器清理失败。请关闭浏览器设置并重试。应用程序已保留。
zh_TW.CleanupFailed=瀏覽器清理失敗。請關閉瀏覽器設定並重試。應用程式已保留。
ms.CleanupFailed=Pembersihan pelayar gagal. Tutup persediaan pelayar dan cuba semula. Aplikasi telah dikekalkan.

ja.ConnectBrowser=Download Engine を開いてブラウザーを接続
ko.ConnectBrowser=Download Engine을 열고 브라우저 연결
vi.ConnectBrowser=Mở Download Engine và kết nối trình duyệt
th.ConnectBrowser=เปิด Download Engine และเชื่อมต่อเบราว์เซอร์
pl.ConnectBrowser=Otwórz Download Engine i połącz przeglądarkę
ru.ConnectBrowser=Открыть Download Engine и подключить браузер
uk.ConnectBrowser=Відкрити Download Engine і підключити браузер
ar.ConnectBrowser=فتح Download Engine وتوصيل متصفح
ja.PortableFolder=これは Download Engine のポータブルフォルダーです。キューとダウンロードを保持するため、別の保存先を選択してください。
ko.PortableFolder=Download Engine 포터블 폴더입니다. 대기열과 다운로드를 보존하려면 다른 설치 폴더를 선택하세요.
vi.PortableFolder=Đây là thư mục Download Engine di động. Chọn thư mục cài khác để giữ hàng đợi và tệp tải xuống.
th.PortableFolder=นี่คือโฟลเดอร์ Download Engine แบบพกพา เลือกติดตั้งที่อื่นเพื่อเก็บคิวและไฟล์ดาวน์โหลด
pl.PortableFolder=To folder przenośnego Download Engine. Wybierz inny folder instalacji, aby zachować kolejki i pobrania.
ru.PortableFolder=Это папка переносного Download Engine. Выберите другую папку для сохранения очередей и загрузок.
uk.PortableFolder=Це папка переносного Download Engine. Виберіть іншу папку, щоб зберегти черги та завантаження.
ar.PortableFolder=هذا مجلد Download Engine محمول. اختر مجلد تثبيت مختلفًا للحفاظ على القائمة والتنزيلات.
ja.OtherCopy=このフォルダーには別の Download Engine があります。別の保存先を選択してください。
ko.OtherCopy=이 폴더에는 다른 Download Engine 사본이 있습니다. 다른 설치 폴더를 선택하세요.
vi.OtherCopy=Thư mục này có bản Download Engine khác. Chọn thư mục cài đặt khác.
th.OtherCopy=โฟลเดอร์นี้มี Download Engine อีกสำเนา เลือกโฟลเดอร์ติดตั้งอื่น
pl.OtherCopy=Ten folder zawiera inną kopię Download Engine. Wybierz inny folder instalacji.
ru.OtherCopy=В этой папке другая копия Download Engine. Выберите другую папку установки.
uk.OtherCopy=У цій папці інша копія Download Engine. Виберіть іншу папку встановлення.
ar.OtherCopy=يحتوي هذا المجلد على نسخة أخرى من Download Engine. اختر مجلد تثبيت مختلفًا.
ja.MissingCleanup=ブラウザーの解除処理がありません。アンインストール前に修復してください。
ko.MissingCleanup=브라우저 정리 도구가 없습니다. 제거 전에 설치를 복구하세요.
vi.MissingCleanup=Thiếu công cụ dọn trình duyệt. Sửa cài đặt trước khi gỡ.
th.MissingCleanup=ไม่มีเครื่องมือล้างการเชื่อมต่อเบราว์เซอร์ ซ่อมการติดตั้งก่อนถอน
pl.MissingCleanup=Brak narzędzia czyszczenia przeglądarki. Napraw instalację przed odinstalowaniem.
ru.MissingCleanup=Средство очистки браузера отсутствует. Восстановите установку перед удалением.
uk.MissingCleanup=Засіб очищення браузера відсутній. Відновіть встановлення перед видаленням.
ar.MissingCleanup=أداة تنظيف المتصفح مفقودة. أصلح التثبيت قبل إزالته.
ja.CannotCleanup=ブラウザーの解除処理を開始できません。アンインストール前に修復してください。
ko.CannotCleanup=브라우저 정리를 시작할 수 없습니다. 제거 전에 설치를 복구하세요.
vi.CannotCleanup=Không khởi động được công cụ dọn trình duyệt. Sửa cài đặt trước khi gỡ.
th.CannotCleanup=เริ่มล้างการเชื่อมต่อเบราว์เซอร์ไม่ได้ ซ่อมการติดตั้งก่อนถอน
pl.CannotCleanup=Nie można uruchomić czyszczenia przeglądarki. Napraw instalację przed odinstalowaniem.
ru.CannotCleanup=Не удалось запустить очистку браузера. Восстановите установку перед удалением.
uk.CannotCleanup=Не вдалося запустити очищення браузера. Відновіть встановлення перед видаленням.
ar.CannotCleanup=تعذر بدء تنظيف المتصفح. أصلح التثبيت قبل إزالته.
ja.CleanupFailed=ブラウザーの解除に失敗しました。ブラウザー設定を閉じて再試行してください。アプリは保持されています。
ko.CleanupFailed=브라우저 정리에 실패했습니다. 브라우저 설정을 닫고 다시 시도하세요. 앱은 유지됩니다.
vi.CleanupFailed=Dọn trình duyệt thất bại. Đóng thiết lập trình duyệt và thử lại. Ứng dụng được giữ lại.
th.CleanupFailed=ล้างการเชื่อมต่อเบราว์เซอร์ล้มเหลว ปิดการตั้งค่าเบราว์เซอร์แล้วลองใหม่ แอปยังคงอยู่
pl.CleanupFailed=Czyszczenie przeglądarki nie powiodło się. Zamknij ustawienia przeglądarki i ponów. Aplikacja została zachowana.
ru.CleanupFailed=Очистка браузера не удалась. Закройте настройки браузера и повторите. Приложение сохранено.
uk.CleanupFailed=Очищення браузера не вдалося. Закрийте налаштування браузера й повторіть. Застосунок збережено.
ar.CleanupFailed=فشل تنظيف المتصفح. أغلق إعداد المتصفح وأعد المحاولة. تم الاحتفاظ بالتطبيق.

de.ConnectBrowser=Download Engine öffnen und einen Browser verbinden
de.PortableFolder=Dies ist ein portabler Download-Engine-Ordner. Wählen Sie einen anderen Installationsordner, um Warteschlange und Downloads zu erhalten.
de.OtherCopy=Dieser Ordner enthält eine andere Download-Engine-Kopie. Wählen Sie einen anderen Installationsordner.
de.MissingCleanup=Browserbereinigung fehlt. Reparieren Sie die Installation vor dem Deinstallieren.
de.CannotCleanup=Browserbereinigung konnte nicht starten. Reparieren Sie die Installation vor dem Deinstallieren.
de.CleanupFailed=Browserbereinigung fehlgeschlagen. Schließen Sie die Browsereinrichtung und versuchen Sie es erneut. Die App wurde beibehalten.
fr.ConnectBrowser=Ouvrir Download Engine et connecter un navigateur
fr.PortableFolder=Ce dossier contient Download Engine portable. Choisissez un autre dossier pour préserver sa file et ses téléchargements.
fr.OtherCopy=Ce dossier contient une autre copie de Download Engine. Choisissez un autre dossier.
fr.MissingCleanup=Nettoyage du navigateur absent. Réparez l’installation avant de désinstaller.
fr.CannotCleanup=Impossible de lancer le nettoyage du navigateur. Réparez l’installation avant de désinstaller.
fr.CleanupFailed=Échec du nettoyage du navigateur. Fermez sa configuration et réessayez. L’application est conservée.
sv.ConnectBrowser=Öppna Download Engine och anslut en webbläsare
sv.PortableFolder=Detta är en portabel Download Engine-mapp. Välj en annan installationsmapp för att bevara kön och hämtningarna.
sv.OtherCopy=Mappen innehåller en annan Download Engine-kopia. Välj en annan installationsmapp.
sv.MissingCleanup=Webbläsarrensning saknas. Reparera installationen före avinstallation.
sv.CannotCleanup=Kunde inte starta webbläsarrensningen. Reparera installationen före avinstallation.
sv.CleanupFailed=Webbläsarrensningen misslyckades. Stäng webbläsarinställningen och försök igen. Appen har behållits.
no.ConnectBrowser=Åpne Download Engine og koble til en nettleser
no.PortableFolder=Dette er en portabel Download Engine-mappe. Velg en annen installasjonsmappe for å bevare køen og nedlastingene.
no.OtherCopy=Mappen inneholder en annen Download Engine-kopi. Velg en annen installasjonsmappe.
no.MissingCleanup=Nettleseropprydding mangler. Reparer installasjonen før avinstallering.
no.CannotCleanup=Kunne ikke starte nettleseropprydding. Reparer installasjonen før avinstallering.
no.CleanupFailed=Nettleseropprydding mislyktes. Lukk nettleseroppsettet og prøv igjen. Appen er beholdt.
de_CH.ConnectBrowser=Download Engine öffnen und einen Browser verbinden
de_CH.PortableFolder=Dies ist ein portabler Download-Engine-Ordner. Wählen Sie einen anderen Installationsordner, um Warteschlange und Downloads zu erhalten.
de_CH.OtherCopy=Dieser Ordner enthält eine andere Download-Engine-Kopie. Wählen Sie einen anderen Installationsordner.
de_CH.MissingCleanup=Browserbereinigung fehlt. Reparieren Sie die Installation vor dem Deinstallieren.
de_CH.CannotCleanup=Browserbereinigung konnte nicht starten. Reparieren Sie die Installation vor dem Deinstallieren.
de_CH.CleanupFailed=Browserbereinigung fehlgeschlagen. Schliessen Sie die Browsereinrichtung und versuchen Sie es erneut. Die App wurde beibehalten.

it.ConnectBrowser=Apri Download Engine e collega un browser
it.PortableFolder=Questa è una cartella portatile di Download Engine. Scegli un’altra cartella di installazione per conservare la coda e i download.
it.OtherCopy=Questa cartella contiene un’altra copia di Download Engine. Scegli un’altra cartella di installazione.
it.MissingCleanup=Manca la pulizia del browser. Ripara questa installazione prima di disinstallare.
it.CannotCleanup=Impossibile avviare la pulizia del browser. Ripara questa installazione prima di disinstallare.
it.CleanupFailed=Pulizia del browser non riuscita. Chiudi Configurazione browser e riprova. L’applicazione è stata conservata.
rm.ConnectBrowser=Avrir Download Engine e colliar in navigatur
rm.PortableFolder=Quest è in ordinatur portabel da Download Engine. Tscherna in auter ordinatur d’installaziun per conservar sia colonna e sias telechargiadas.
rm.OtherCopy=Quest ordinatur cuntegna in’autra copia da Download Engine. Tscherna in auter ordinatur d’installaziun.
rm.MissingCleanup=La nettegiada dal navigatur manca. Reparescha questa installaziun avant deinstallar.
rm.CannotCleanup=Impussibel d’aviar la nettegiada dal navigatur. Reparescha questa installaziun avant deinstallar.
rm.CleanupFailed=Nettegiada dal navigatur betg reussida. Serra la Configuraziun dal navigatur ed emprova danovamain. L’applicaziun è restada.

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Files]
Source: "{#SourceRoot}\installer\languages\it.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: it; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\rm.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: rm; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\de.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: de; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\fr.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: fr; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\sv.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: sv; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\no.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: no; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\de-CH.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: de_CH; Flags: ignoreversion
Source: "{#SourceRoot}\target\release\de.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceRoot}\target\release\de-host.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceRoot}\target\release\de-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceRoot}\installer\installed.json"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\en.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: en; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\zh-CN.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: zh_CN; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\zh-TW.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: zh_TW; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\ms.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: ms; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\ja.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: ja; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\ko.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: ko; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\vi.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: vi; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\th.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: th; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\pl.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: pl; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\ru.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: ru; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\uk.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: uk; Flags: ignoreversion
Source: "{#SourceRoot}\installer\languages\ar.json"; DestDir: "{app}"; DestName: "installer-language.json"; Languages: ar; Flags: ignoreversion
Source: "{#SourceRoot}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceRoot}\SECURITY.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceRoot}\PRIVACY.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceRoot}\CHANGELOG.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceRoot}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceRoot}\THIRD-PARTY-NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceRoot}\docs\licenses\*"; DestDir: "{app}\docs\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#SourceRoot}\docs\github-release.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#SourceRoot}\docs\reference.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#SourceRoot}\docs\mega.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#SourceRoot}\docs\wetransfer.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#SourceRoot}\docs\images\*"; DestDir: "{app}\docs\images"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#SourceRoot}\tools\ffmpeg\source\*"; DestDir: "{app}\tools\ffmpeg\source"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#SourceRoot}\tools\ffmpeg\licenses\*"; DestDir: "{app}\tools\ffmpeg\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#SourceRoot}\LICENSE"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\LICENSE"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\LICENSE"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\LICENSE"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\PRIVACY.md"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\PRIVACY.md"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\PRIVACY.md"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\PRIVACY.md"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\tools\ffmpeg\ffmpeg.exe"; DestDir: "{app}\tools\ffmpeg"; Flags: ignoreversion
Source: "{#SourceRoot}\tools\ffmpeg\LICENSE"; DestDir: "{app}\tools\ffmpeg"; Flags: ignoreversion
Source: "{#SourceRoot}\tools\ffmpeg\README.txt"; DestDir: "{app}\tools\ffmpeg"; Flags: ignoreversion
Source: "{#SourceRoot}\tools\ffmpeg\PROVENANCE.md"; DestDir: "{app}\tools\ffmpeg"; Flags: ignoreversion
Source: "{#SourceRoot}\tools\ffmpeg\media-manifest.json"; DestDir: "{app}\tools\ffmpeg"; Flags: ignoreversion
Source: "{#SourceRoot}\scripts\Register-Browsers.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\core.js"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\capture.js"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\github-download.js"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\link-recovery.js"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\media.js"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\permission-flow.js"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\background.js"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\popup.html"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\popup.css"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\popup.js"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\browser-login.js"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\login.html"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\login.js"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\page-links.js"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\video-button-manager.js"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\video-button.js"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\links.html"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\links.css"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\links.js"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\manifest.chromium.json"; DestDir: "{app}\extensions\chrome"; DestName: "manifest.json"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\core.js"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\capture.js"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\github-download.js"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\link-recovery.js"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\media.js"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\permission-flow.js"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\background.js"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\popup.html"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\popup.css"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\popup.js"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\browser-login.js"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\login.html"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\login.js"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\page-links.js"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\video-button-manager.js"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\video-button.js"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\links.html"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\links.css"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\links.js"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\manifest.chromium.json"; DestDir: "{app}\extensions\edge"; DestName: "manifest.json"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\core.js"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\capture.js"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\github-download.js"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\link-recovery.js"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\media.js"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\permission-flow.js"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\background.js"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\popup.html"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\popup.css"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\popup.js"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\browser-login.js"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\login.html"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\login.js"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\page-links.js"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\video-button-manager.js"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\video-button.js"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\links.html"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\links.css"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\links.js"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\manifest.chromium.json"; DestDir: "{app}\extensions\brave"; DestName: "manifest.json"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\core.js"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\capture.js"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\github-download.js"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\link-recovery.js"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\media.js"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\permission-flow.js"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\background.js"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\popup.html"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\popup.css"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\popup.js"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\browser-login.js"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\login.html"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\login.js"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\page-links.js"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\video-button-manager.js"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\video-button.js"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\links.html"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\links.css"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\links.js"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\manifest.firefox.json"; DestDir: "{app}\extensions\firefox"; DestName: "manifest.json"; Flags: ignoreversion

Source: "{#SourceRoot}\docs\browser-release.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#SourceRoot}\docs\youtube-port.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#SourceRoot}\docs\automation.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#SourceRoot}\examples\core-api-automation.lua"; DestDir: "{app}\examples"; Flags: ignoreversion
Source: "{#SourceRoot}\examples\custom-site-reader.lua"; DestDir: "{app}\examples"; Flags: ignoreversion
Source: "{#SourceRoot}\examples\download-provider.lua"; DestDir: "{app}\examples"; Flags: ignoreversion
Source: "{#SourceRoot}\examples\download-provider-custom-ctr.lua"; DestDir: "{app}\examples"; Flags: ignoreversion
Source: "{#SourceRoot}\docs\lua-providers.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#SourceRoot}\docs\lua-provider-lab.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#SourceRoot}\docs\mcp.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#SourceRoot}\docs\mcp-ai-guide.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#SourceRoot}\docs\user-guide.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#SourceRoot}\docs\releases.md"; DestDir: "{app}\docs"; Flags: ignoreversion

Source: "{#SourceRoot}\extension\i18n-catalog.js"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\i18n.js"; DestDir: "{app}\extensions\chrome"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\_locales\*"; DestDir: "{app}\extensions\chrome\_locales"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#SourceRoot}\extension\icons\*"; DestDir: "{app}\extensions\chrome\icons"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#SourceRoot}\extension\i18n-catalog.js"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\i18n.js"; DestDir: "{app}\extensions\edge"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\_locales\*"; DestDir: "{app}\extensions\edge\_locales"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#SourceRoot}\extension\icons\*"; DestDir: "{app}\extensions\edge\icons"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#SourceRoot}\extension\i18n-catalog.js"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\i18n.js"; DestDir: "{app}\extensions\brave"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\_locales\*"; DestDir: "{app}\extensions\brave\_locales"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#SourceRoot}\extension\icons\*"; DestDir: "{app}\extensions\brave\icons"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#SourceRoot}\extension\i18n-catalog.js"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\i18n.js"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion
Source: "{#SourceRoot}\extension\_locales\*"; DestDir: "{app}\extensions\firefox\_locales"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#SourceRoot}\extension\icons\*"; DestDir: "{app}\extensions\firefox\icons"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#SourceRoot}\docs\localization.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#SourceRoot}\docs\locales\*"; DestDir: "{app}\docs\locales"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
#ifndef TestBuild
Name: "{autoprograms}\Download Engine"; Filename: "{app}\de.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\Download Engine"; Filename: "{app}\de.exe"; WorkingDir: "{app}"; Tasks: desktopicon
#endif

[InstallDelete]
; Programs and shortcuts of releases before 2.0, replaced by de*.exe above.
Type: files; Name: "{app}\idm-ex.exe"
Type: files; Name: "{app}\idm-ex-host.exe"
Type: files; Name: "{app}\idm-ex-cli.exe"
#ifndef TestBuild
Type: files; Name: "{autoprograms}\IDM EX.lnk"
Type: files; Name: "{autodesktop}\IDM EX.lnk"
#endif

[Run]
Filename: "{app}\de.exe"; Parameters: "--browser-setup"; Description: "{cm:ConnectBrowser}"; Flags: postinstall nowait skipifsilent

[UninstallDelete]
; Exact app-owned manifests only. Never recursively remove user data or downloads.
Type: files; Name: "{app}\native-download-engine-chrome.json"
Type: files; Name: "{app}\native-download-engine-edge.json"
Type: files; Name: "{app}\native-download-engine-brave.json"
Type: files; Name: "{app}\native-download-engine-firefox.json"
Type: files; Name: "{app}\native-chrome.json"
Type: files; Name: "{app}\native-edge.json"
Type: files; Name: "{app}\native-brave.json"
Type: files; Name: "{app}\native-firefox.json"

[Code]
function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if FileExists(ExpandConstant('{app}\portable.json')) then
    Result := CustomMessage('PortableFolder')
  else if (FileExists(ExpandConstant('{app}\de.exe')) or FileExists(ExpandConstant('{app}\idm-ex.exe'))) and
    not FileExists(ExpandConstant('{app}\installed.json')) then
    Result := CustomMessage('OtherCopy');
end;

{ "Start with Windows" keeps its pre-2.0 value name, shared by every copy.
  The app always writes a quoted path, so a prefix match identifies the program. }
function StartupStartsProgram(const Command, ProgramPath: String): Boolean;
begin
  Result := Pos(Lowercase('"' + ExpandConstant(ProgramPath) + '"'), Lowercase(Command)) = 1;
end;

{ Removes "Start with Windows" only when it starts this installation;
  another Download Engine copy keeps its entry. }
procedure RemoveStartupEntry;
var
  Command: String;
begin
  if RegQueryStringValue(HKCU, '{#StartupRunKey}', 'XDownloader', Command) and
    (StartupStartsProgram(Command, '{app}\de.exe') or StartupStartsProgram(Command, '{app}\idm-ex.exe')) then
  begin
    RegDeleteValue(HKCU, '{#StartupRunKey}', 'XDownloader');
    RegDeleteValue(HKCU, '{#StartupApprovedKey}', 'XDownloader');
  end;
end;

{ An upgrade from before 2.0 removes idm-ex*.exe. Point this installation's
  startup entry and browser connections at the new programs right away, so
  they work before the app is opened again. The app repeats both at launch. }
procedure MigrateLegacyPrograms;
var
  Command: String;
  ResultCode: Integer;
begin
  if RegQueryStringValue(HKCU, '{#StartupRunKey}', 'XDownloader', Command) and
    StartupStartsProgram(Command, '{app}\idm-ex.exe') then
  begin
    Delete(Command, 1, Length('"' + ExpandConstant('{app}\idm-ex.exe') + '"'));
    RegWriteStringValue(HKCU, '{#StartupRunKey}', 'XDownloader', '"' + ExpandConstant('{app}\de.exe') + '"' + Command);
  end;
  if Exec(ExpandConstant('{app}\de-host.exe'), '--migrate-browser-registrations', ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ResultCode) and (ResultCode = 0) then
    Log('Browser connection migration finished.')
  else
    Log('Browser connections were not migrated; the app retries at launch.');
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    MigrateLegacyPrograms;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ResultCode: Integer;
begin
  if CurUninstallStep <> usUninstall then Exit;
  { Runs after confirmation and the mutex check, before any files are removed. }
  if not FileExists(ExpandConstant('{app}\de-host.exe')) then
    RaiseException(CustomMessage('MissingCleanup'));
  if not Exec(ExpandConstant('{app}\de-host.exe'), '--unregister-all-browsers', ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ResultCode) then
    RaiseException(CustomMessage('CannotCleanup'));
  if ResultCode <> 0 then
    RaiseException(CustomMessage('CleanupFailed'));
  RemoveStartupEntry;
end;
