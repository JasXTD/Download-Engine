# Download Engine

## Pierwsze kroki

Preferencje otwierają się w oknie z kategoriami. Zapisz stosuje zmiany; Anuluj, × lub Escape odrzuca niezapisane zmiany. Motyw można podejrzeć, a Reset przywraca tylko bieżącą kategorię. Edytory zaawansowane zapisują osobno i po zamknięciu wracają do Preferencji.

### Pierwsze kroki

Wybierz Dodaj URL, wklej bezpośredni link do pliku, sprawdź nazwę, kolejkę i folder, a następnie rozpocznij. Dodaj grupę przyjmuje jeden link w wierszu. Z wzorca tworzy numerowane linki, np. img001…img100, z jednego linku z *. Język wybierzesz w Ustawienia → Wygląd → Język. Język systemu korzysta z Windows; nieobsługiwany język zastępuje angielski.

## Pobieranie i odzyskiwanie

### Pobieranie i odzyskiwanie

Kliknij pobranie dwukrotnie, aby zobaczyć postęp, prędkość i pozostały czas. Wstrzymanie i zatrzymanie zachowują dane. Rozpocznij teraz daje pierwszeństwo wybranym plikom. Wznawianie wymaga obsługi serwera i zgodności pliku. Wygasły link odświeżysz w menu kontekstowym. Wyczyść ukończone usuwa tylko historię, pozostawiając pliki. Zadania → Eksportuj listę… zapisuje pobrania w pliku jako kopię zapasową lub do przeniesienia na inny komputer. Zadania → Importuj listę… pokazuje nowe pozycje przed dodaniem; linki już obecne są pomijane, a nowe pobrania pozostają wstrzymane. Widok → Pływające pole upuszczania pokazuje małe pole zawsze nad innymi oknami; upuść na nim linki, aby je pobrać, także gdy Download Engine jest w zasobniku.

## Witryny offline

### Witryny offline

Grabber wyszukuje pliki lub zapisuje witrynę offline. Ustaw URL, głębokość, liczbę stron i rozmiar; sprawdź wyniki przed dodaniem. Otwórz zapisany index.html. Funkcje dynamiczne, logowanie, formularze i zaszyfrowane filmy nie są obsługiwane. Braki są opisane w raporcie.

## Integracja z przeglądarką

### Integracja z przeglądarką

Otwórz Narzędzia → Ustawienia przeglądarki, aby połączyć Chrome, Edge, Brave lub Firefox. Chrome, Edge i Brave wymagają rzeczywistego ID zainstalowanego rozszerzenia. Użyj sprawdzenia połączenia w oknie rozszerzenia. Opcje przeglądarki mają osobny język. Przycisk filmu otwiera wybór jakości. Logowanie witryny jest używane tylko dla danego pliku po Twojej zgodzie.

## Kolejki i harmonogramy

### Kolejki i harmonogramy

Zarządzaj kolejkami, aby grupować pliki i ustawiać liczbę jednoczesnych pobrań. Nowe kolejki czekają na uruchomienie lub harmonogram. Harmonogram obsługuje start, zatrzymanie i powtarzanie. Aplikacja musi działać, także w zasobniku. Zakończenie aplikacji oraz wyłączenie, uśpienie lub hibernacja komputera wymagają osobnego włączenia; odliczanie można anulować.

## Skrypty

### Skrypty

W Narzędzia → Skrypty utwórz reguły według witryny lub rozszerzenia. W skryptach wybierz Lua lub PowerShell, zapisz i uruchom. Wyzwalacze: ręczny, start aplikacji, interwał, ukończenie pobrania i kolejki. Skrypt nie nakłada uruchomień na siebie. Zatrzymanie kończy skrypt, ale nie cofa wysłanych poleceń pobierania. Wyniki znajdziesz w dzienniku.

## Skrypty Lua

### Skrypty Lua

Lua jest wbudowane. Utwórz skrypt Lua z ręcznym wyzwalaczem, zapisz i uruchom poniższy przykład. xd.queues() zwraca kolejki, xd.downloads() pobrania. Ukończenie pobrania udostępnia event.downloads, a kolejki event.queue. Nie tłumacz nazw poleceń i pól. Pełne API: docs/automation.md. Nie zapisuj sekretów w dziennikach. PowerShell działa z uprawnieniami Twojego konta Windows.

Edytory Lua i PowerShell automatycznie sprawdzają składnię po krótkiej przerwie w pisaniu, bez zapisywania ani uruchamiania skryptu. Dodano numery wierszy, kolory składni i klikalne błędy. Wpisz początek nazwy lub naciśnij Ctrl+Space, wybierz strzałkami, wstaw klawiszem Tab, Enter lub kliknięciem i zamknij klawiszem Esc. Uzupełnienia można cofnąć; błędy wykonania pozostają w dzienniku.

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

Wyzwalacz Pobieranie nieudane uruchamia skrypt, gdy pobranie nadal wymaga uwagi po automatycznych ponowieniach Download Engine; event.downloads zawiera wtedy error każdego pobrania. xd.retry(id) i xd.restart(id) próbują ponownie (skrypt może to zrobić 5 razy dla jednego pobrania), xd.notify(title, text) pokazuje powiadomienie Windows, a xd.store.get i xd.store.set przechowują wartości między uruchomieniami. xd.rename, xd.move i xd.remove zmieniają plik tylko wtedy, gdy zaznaczono Może zmieniać pobrane pliki. Uruchom z próbką… testuje skrypt zdarzenia na wybranych pobraniach; zmiany są tylko symulowane.

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

Aby edytować skrypt w innym programie, użyj przycisków Otwórz plik… i Zapisz do pliku… w nagłówku edytora. Otwórz plik… kopiuje plik .lua lub .ps1 do edytora i pyta najpierw, jeśli zastąpiłby niezapisany kod; wybierz Zapisz skrypt, aby go zachować. Zapisz do pliku… zapisuje kod z edytora do pliku.

## Wiersz poleceń

### Wiersz poleceń

Otwórz PowerShell w folderze aplikacji i użyj poniższych poleceń. Aplikacja musi działać, aby przyjmować sterowanie. JSON i nazwy poleceń pozostają angielskie. Ścieżki ze spacjami ujmuj w cudzysłowy.

```powershell
.\de-cli.exe --help
.\de-cli.exe queues
.\de-cli.exe list
```

## Integracja MCP

### Integracja MCP

Na stronie MCP uruchom lokalny serwer HTTP i skopiuj konfigurację klienta. Zacznij od trybu tylko do odczytu. Klient AI potrzebuje URL i prywatnego nagłówka uwierzytelniania. Nie publikuj tokenu. Zatrzymaj serwer lub zmień token, aby cofnąć dostęp. Szczegóły: docs/mcp-ai-guide.md.

MCP otwiera się teraz w oknie z paska narzędzi, menu Narzędzia lub Preferencji, bez zmiany bieżącej strony. Zamknięcie przez × lub Escape nie zatrzymuje serwera i zachowuje niezapisane zmiany MCP. Aby odłączyć klientów AI, użyj zatrzymania serwera. Po otwarciu z Preferencji zamknięcie przywraca te same oczekujące zmiany.

## Skanowanie antywirusowe

Skaner: Microsoft Defender, Malwarebytes, Kaspersky, Bitdefender, ClamAV.

Wymaga angielskiej integracji programie antywirusowym z Eksploratorem. Limit dotyczy przekazania, nie skanu. Anuluj skan w programie antywirusowym.

Wymagany jest domyślny folder instalacji.

Ochroną i kwarantanną zarządza antywirus. Wysłanie żądania nie oznacza braku zagrożeń.

Sprawdź konfigurację: Znaleziono integrację; dostępność skanowania jest sprawdzana przy żądaniu.

### Skanowanie antywirusowe

Otwórz Narzędzia → Skan antywirusowy. Pojedynczy pozwala wybrać jeden ukończony plik, a Grupowy wiele plików. Pojedynczy skan można anulować. Karty są zablokowane podczas skanowania lub przeglądu wyników Malwarebytes / Kaspersky / Bitdefender.

Wybierz skaner w ustawieniach, potem Narzędzia → Skan antywirusowy → Grupowy. Zaznacz ukończone pliki, rozpocznij i sprawdź wyniki. Zatrzymanie anuluje niewysłane żądania. Malwarebytes / Kaspersky / Bitdefender czeka na Twoją ocenę po każdym żądaniu. Przekazane skany zatrzymaj w Malwarebytes / Kaspersky / Bitdefender. Wysłanie żądania nie oznacza, że plik jest bezpieczny.

## Ustawienia i dzienniki

### Ustawienia i dzienniki

Ustawienia obejmują język, motyw, foldery, prędkość, limity danych, proxy i liczbę połączeń. Dzienniki można przeszukiwać, filtrować, kopiować i eksportować. URL, ścieżki, wynik skryptu i błędy techniczne zachowują pierwotny tekst. Sprawdź raport przed udostępnieniem.

Wersja zainstalowana przy każdym uruchomieniu sprawdza bieżący folder Pobrane systemu Windows, także po przeniesieniu go na inny dysk. Nowe pobrania korzystają z tej lokalizacji. Istniejące pobrania, zapisany postęp i własne foldery pozostają bez zmian. Wersja przenośna za każdym razem ustala folder względem aplikacji. Starsze profile zachowują nazwę podfolderu IDM EX. Brak możliwości odczytania lokalizacji powoduje wyświetlenie błędu.

## Rozwiązywanie problemów

### Rozwiązywanie problemów

Przy braku połączenia sprawdź, czy aplikacja działa, ID rozszerzenia i rejestrację. Po aktualizacji odśwież rozszerzenie i stronę. Błędy pobierania: sprawdź miejsce na dysku, prawa folderu, proxy i ważność linku. Nie usuwaj danych odzyskiwania przed próbą wznowienia. Jeśli litery są kwadratami, dodaj czcionki języka w Windows i uruchom aplikację ponownie.
