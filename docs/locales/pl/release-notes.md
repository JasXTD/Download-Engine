## 3.19.0 — 2026-11-08

- Obsługa torrentów v2 i hybrydowych z weryfikacją SHA-256, wyborem plików, wznawianiem i odzyskiwaniem. Obsługa v1 oraz ustawienia udostępniania pozostają bez zmian.

[English details](../../../CHANGELOG.md#3190--2026-11-08)

## 3.18.1 — 2026-11-08

- Wysyłanie jest domyślnie wyłączone. Możesz je włączyć w Preferencje → Torrenty.
- **Zezwolić na połączenia przychodzące?** Inne peery będą mogły łączyć się z aplikacją. Zwiększa to ekspozycję sieciową. Ustawienia zapory i routera pozostaną bez zmian.
- **Włączyć UPnP?** Podczas pracy torrentów aplikacja poprosi router o przekierowanie portu TCP torrenta. Inne peery mogą wtedy uzyskać dostęp do aplikacji z internetu. Zapisz Preferencje, aby zastosować tę zmianę.

[English details](../../../CHANGELOG.md#3181--2026-11-08)

## 3.18.0 — 2026-11-08

### VPN / interfejs sieciowy

- Przypisz torrenty do jednego adresu interfejsu. Gdy zniknie, transfery się zatrzymają; inne pobrania zachowają własne ustawienia.
- DNS również używa tego interfejsu. Wybierz tę samą wersję IP co adres interfejsu. Mapowanie portów routera jest wyłączone podczas przypisania.
- TCP / uTP · Szyfrowane / Nieszyfrowane

[English details](../../../CHANGELOG.md#3180--2026-11-08)

## 3.17.0 — 2026-11-08

### Szyfrowanie połączeń, uTP i źródła HTTP

Dodano opcjonalne szyfrowanie MSE/PE, połączenia uTP i źródła web seed HTTP/HTTPS. Wszystkie trzy opcje są domyślnie wyłączone i można je włączyć w ustawieniach torrentów. Fragmenty są weryfikowane, a źródła zapisywane, aby umożliwić wznowienie po ponownym uruchomieniu. Poprawiono wstrzymywanie, wznawianie i zatrzymywanie wszystkich transferów. Szyfrowanie nie ukrywa adresu IP.

[English details](../../../CHANGELOG.md#3170--2026-11-08)

## 3.16.1 — 2026-11-08

### Poprawki ustawień i sterowania

Ustawienia rozszerzeń zapisują teraz pusty tekst i cały formularz w jednej transakcji. Niezapisane zmiany pozostają po przełączeniu rozszerzenia. Przycisk zapisu jest widoczny w małych oknach, a aktualizacje odświeżają wartości domyślne. Naprawiono menu zatrzymywania seedowanych torrentów i widok limitów prędkości, ograniczono powtarzające się zapisy postępu. Uproszczono etykietę eksportu raportu i poprawiono opisy ochrony systemowej oraz ustawień aktualizacji.

[English details](../../../CHANGELOG.md#3161--2026-11-08)

## 3.16.0 — 2026-11-07

### Przykładowe rozszerzenia Lua i zarządzanie uprawnieniami

Dołączono pięć rozszerzeń Lua: publiczne pliki Internet Archive, oryginały Wikimedia Commons, zasoby wydań GitHub, sortowanie ukończonych plików według typu oraz powiadomienia o zakończeniu kolejki z opcjonalnym uruchomieniem następnej. Wybierz je w Extensions → Examples i sprawdź uprawnienia przed instalacją; samo dołączenie do aplikacji nie uruchamia kodu. Ulepszono pustą stronę rozszerzeń, przegląd instalacji i uprawnienia zadań. Dodano nadawanie i odbieranie dostępu do plików po instalacji oraz poradnik pierwszego rozszerzenia.

Nieaktualny przegląd nie pozwala na instalację zmienionego pakietu. Zastrzeżone identyfikatory przykładów wymagają dokładnej zgodności dołączonych plików, a metadane odrzucają mylące znaki sterujące. Rozszerzenia nadal są wykrywane w folderze plugins aplikacji. Uzupełniono tłumaczenia raportów awarii i interfejsu, poprawiono wyszukiwanie tekstów do tłumaczenia i ujednolicono język romansz. Przywrócono brakujące historyczne informacje o wydaniach; pakiety macOS/Linux zawierają poradniki i obrazy potrzebne w dokumentacji offline.

[English details](../../../CHANGELOG.md#3160--2026-11-07)

## 3.15.0 — 2026-11-06

### Rdzeń dostawców Lua i napisy ASS

HTTP w Lua obsługuje teraz HEAD, PUT, PATCH, DELETE i OPTIONS, dane binarne o ograniczonym rozmiarze oraz jawnie przekazywane sesje przypisane do jednego źródła. Poświadczenia pozostają zaszyfrowane poza modułem rozwiązywania adresów i nie trafiają do planów ani procesorów offline. Plany multimediów obsługują segmenty o nieznanym rozmiarze, a lua_stream przetwarza ograniczone porcje danych ze zweryfikowanymi punktami kontrolnymi dla pauzy, wznowienia, zatrzymania i odzyskiwania.

Dodano import poświadczeń binarnych/RSA chronionych przez system operacyjny, losowe bajty, SHA-1/SHA-256, HMAC, podpisywanie i odszyfrowywanie RSA oraz odszyfrowywanie bloków AES. Napisy ASS zachowują style i formatowanie wewnątrz wierszy w MKV, a w MP4 są konwertowane na napisy tekstowe. Nowe funkcje poświadczeń i multimediów wymagają aplikacji obsługującej wersję zapisu profilu 9; dotychczasowe formaty punktów kontrolnych pozostają zgodne.

Laboratorium CENC offline dla autorów używa Lua do analizy ograniczonego podzbioru szyfrowania próbek AVC/AAC, symulowanej wymiany podpisanej licencji, przekazywania kluczy i łączenia odszyfrowanych ścieżek z napisami ASS. Uwierzytelnianie witryn, formaty licencji i parsery kontenerów pozostają zadaniem kodu Lua. Testy wykorzystują wygenerowane dane, bez rzeczywistych usług i kont; nie jest to pełna implementacja CENC.

### Udostępnianie i wyszukiwanie torrentów

Udostępnianie ukończonego torrenta kończy się teraz samo po osiągnięciu współczynnika 2.0 lub po tygodniu, zależnie od tego, co nastąpi wcześniej. Działa to domyślnie, także w istniejących profilach; oba limity można zmienić lub wyłączyć w sekcji Preferencje → Torrenty. Wysyłanie podczas pobierania wlicza się do współczynnika. Okno stanu pokazuje współczynnik, a torrent zatrzymany przez limit zachowuje pliki i ma w menu opcję Udostępnij ponownie.

Linki magnet bez trackera nie są już po prostu odrzucane. Okno Dodaj pobieranie wyjaśnia, że wyszukiwanie w DHT pokazuje innym komputerom szukany torrent i adres IP, i szuka dopiero po kliknięciu Szukaj w DHT. Komputer jest ogłaszany jako peer dopiero wtedy, gdy sprawdzona lista plików potwierdzi publiczny torrent. Prywatny torrent zatrzymuje wyszukiwanie, a wyszukiwanie kończy się po 10 minutach. Linki z przeglądarek, schowka lub Eksploratora nigdy same nie uruchamiają wyszukiwania.

Nowa opcja Skonfiguruj router automatycznie (UPnP) jest domyślnie wyłączona, podobnie jak wymagane przez nią połączenia przychodzące. Komunikuje się tylko z bramą domyślną w sieci prywatnej, prosi o jedną regułę przekierowania ważną godzinę, odnawianą podczas pracy torrentów, i usuwa wyłącznie własną regułę; w macOS jest niedostępna. Znalezione peery i węzły DHT na portach innych usług (poniżej 1024 oraz np. SIP 5060) nigdy nie są kontaktowane. Pobierania zatrzymane z powodu DHT pokazują teraz przyczynę w wybranym języku.

[English details](../../../CHANGELOG.md#3150--2026-11-06)

## 3.10.2 — 2026-11-04

### Lokalne raporty awarii

Pomoc i dzienniki umożliwiają przeglądanie, kopiowanie i eksport raportów Rust panic oraz nieprawidłowego zamknięcia. Brak automatycznego przesyłania; raporty nie zawierają haseł, adresów pobierania, prywatnych ścieżek ani zrzutów pamięci.

[English details](../../../CHANGELOG.md#3102--2026-11-04)

## 3.10.1 — 2026-11-04

### Bezpieczeństwo i niezawodność

Ulepszono limity zasobów podczas wyszukiwania peerów torrent, anulowanie i filtrowanie adresów sieciowych. Zaktualizowano magazyny poświadczeń w macOS i Linuksie, zachowując zapisane loginy, oraz zabezpieczono istniejące lokalizacje przed nadpisaniem podczas zapisywania folderów torrent.

[English details](../../../CHANGELOG.md#3101--2026-11-04)

## 3.10.0 — 2026-11-03

### macOS i Linux

Natywne pakiety dla komputerów Mac z Apple Silicon i Intel oraz archiwum Linux x64 używają tego samego interfejsu i silnika pobierania co Windows. Zawierają FFmpeg, jego źródła i licencje, natywne okna dialogowe i systemowy magazyn poświadczeń. Pakiety Mac mają podpis ad hoc, bez notaryzacji Apple. Wymagania i zakres testów opisano w poradniku.

[Guide / English](../../cross-platform.md)

## 3.9.0 — 2026-11-01

### Obsługa torrentów

Dodano obsługę linków magnet z przeglądarki, przeciąganie plików torrent i propozycje ze schowka. Seedowanie po zakończeniu i połączenia przychodzące są osobnymi opcjami, domyślnie wyłączonymi. Dostępne są skanowanie wielu plików, przenoszenie i zmiana nazw folderów; Zatrzymaj wszystko zatrzymuje też udostępnianie.

[Guide / English](../../user-guide.md#download-torrents)

## 3.8.0 — 2026-11-01

### Otwieranie torrentów z Windows

Otwieraj pliki .torrent z Eksploratora i linki magnet z przeglądarki do sprawdzenia. Opcjonalna rejestracja w Windows zachowuje aplikacje domyślne, a oczekujące pozycje pozostają po ponownym uruchomieniu. Wzmocniono zabezpieczenia torrentów: magnet bez zapisanych metadanych wymaga trackera; w przeciwnym razie użyj pliku .torrent. Dodano testy regresji istniejącej ochrony przed kolizjami nazw plików.

## 3.7.0 — 2026-11-01

### Pobieranie torrentów

Obsługa plików torrent i odnośników magnet z wyborem plików, wstrzymywaniem, wznawianiem i odzyskiwaniem zweryfikowanych danych. Domyślnie wyłączona; włącz ją i ustaw wysyłanie w Preferencjach. Proxy wstrzymuje torrenty, a wysyłanie kończy się po pobraniu. Pakiety zawierają poprawki 3.6.1 oraz program pomocniczy torrentów.

### Integracja wydania

Zaktualizowano poradnik, README, witrynę i kontrole instalatora. Po dodaniu pierwszego torrenta profil wymaga wersji 3.7.0 lub nowszej, aby starsze wersje nie zmieniały zadań torrent. Samo włączenie funkcji nie aktualizuje profilu.

## 3.6.1 — 2026-10-31

### Porządkowanie zależności i zatwierdzanie rozszerzeń

Wbudowane blokady plików Rust zastępują fs2 i usuwają starą zależność winapi, zachowując dotychczasowe blokowanie i oczekiwanie. Aktualizacje rozszerzeń dodające zadania, również przez zmianę identyfikatora zadania, wymagają teraz zgody nawet wtedy, gdy rozszerzenie jest wyłączone.

## 3.6.0 — 2026-10-31

### Rozszerzenia

Nowa strona Rozszerzenia zastępuje Skrypty i ma karty Zainstalowane, Skrypty, Reguły oraz Historia uruchomień. Rozszerzenie to jeden plik .de-extension.json łączący czytniki witryn, dostawców pobierania i zadania automatyzacji. Instalacja jest możliwa tylko z pliku lokalnego, a wydawcy nie są weryfikowani. Okno przeglądu pokazuje, co rozszerzenie może zrobić, po instalacji jest ono wyłączone, a aktualizacja musi poprosić o zgodę na nowy dostęp. Dostęp do sieci jest ograniczony do zadeklarowanych hostów. Zainstalowanie pierwszego rozszerzenia blokuje zapis profilu w starszych wersjach aplikacji.

## 3.5.2 — 2026-10-30

### Sprawdzaj aktualizacje przy uruchomieniu

Domyślnie włączone: jedno sprawdzenie przy każdym otwarciu aplikacji. Nowa wersja pozwala wybrać aktualizację lub odłożyć ją. Błędy sieci nie otwierają okna. Opcję można wyłączyć w Preferencjach. Pobierz i zainstaluj aktualizacje z oficjalnej strony wydań GitHub.

## 3.5.1 — 2026-10-29

### Bezpieczeństwo dostawców Lua

- Resolve/refresh i transform/finish działają w procesach z oddzielnymi uprawnieniami. Proces przetwarzający nie otrzymuje `xd.fetch`, nawet podczas wczytywania kodu. Inicjalizację rozdziela `xd.phase`; lokalne zmienne resolvera nie przechodzą do przetwarzania.
- Zapisany plan zawiera tylko rozmiar, skróty i identyfikator transformacji. Rozwiązane adresy URL są odświeżane w razie potrzeby, a kod przypisany do zadania chroni DPAPI bieżącego użytkownika Windows. Atomowa migracja zachowuje powiązania punktów kontrolnych i blokuje zapis przez starsze wersje.
- `xd.key` i `scripts protect-key`, odczytujący klucz z przekierowanego stdin, udostępniają niejawne uchwyty kluczy AES przypisane do dostawcy. Tajne klucze nie muszą znajdować się w kodzie ani eksportach. Stare kopie zapasowe i klucze zapisane dosłownie w bibliotece nie są usuwane; zobacz `docs/lua-providers.md`.

### Wyczyść historię

Czyszczenie historii ukończonych pobrań wymaga teraz potwierdzenia i pokazuje kolejkę oraz liczbę wpisów. Anulowanie lub zamknięcie zachowuje historię. Pobrane pliki nie są usuwane.

## 3.5.0 — 2026-10-29

### Lua Provider API 1

Dostawcy pobierania Lua obsługują rozwiązywanie źródeł, przetwarzanie bajtów, weryfikację skrótów i zwykłe wstrzymywanie/wznawianie. Każde zadanie zachowuje zapisaną wersję skryptu. Testy używają wyłącznie lokalnie wygenerowanych zaszyfrowanych plików. Zapisanie dostawcy blokuje zapis profilu w starszych wersjach aplikacji.

## 3.4.1 — 2026-10-29

Lista ogranicza kopiowanie rekordów i powtarzanie wyszukiwania w dużych kolejkach. Małe okna zapewniają więcej miejsca na nazwy plików. Kolejkę można uruchomić lub wstrzymać obok jej stanu; uruchomienie zastępuje harmonogram. Po najechaniu na stan widać informacje o ponawianiu i błędach. Transfer, weryfikacja i format wznawiania pozostają bez zmian.

## 3.4.0 — 2026-10-29

### Kolekcja motywów

Preferencje → Wygląd → Własny oferuje Neon Courier, Reactor Night, Aurora i Moonveil. Dostosuj tło i akcent lub zaimportuj PNG/JPEG. Zapisz zachowuje podgląd; Anuluj przywraca wygląd. Obrazy pozostają lokalne, mają limity rozmiaru i są przetwarzane w tle.

### Ulepszenia profili i sterowania

Wersja zawiera też ochronę wersji profilu i kontrolę dostępu MCP.

## 3.3.3 — 2026-10-28

### Ulepszenia interfejsu

Edytor i wczytane skrypty mają teraz osobne karty. Skrypty można wyszukiwać i tworzyć na liście, a pełny edytor otworzyć z nagłówka Lua lub PowerShell. Zapis, uruchamianie/zatrzymywanie i opcjonalne dzienniki znajdują się przy kodzie. Polecenia plików przeniesiono do menu działań edytora. Zmiana widoku zachowuje niezapisany kod, ustawienia i pozycję kursora.

### Sprawniejsze działania pobierania

Odświeżanie kolejek i stanu, obsługa filmów z przeglądarki oraz dodawanie pobrań odbywają się w tle, aby wolny dysk nie blokował kliknięć. Okna dodawania pokazują postęp, zapobiegają wielokrotnemu wysłaniu i zachowują dane po błędzie zapisu. Foldery również otwierają się w tle, a stare wyniki odświeżania nie zastępują nowo wybranej kolejki.
