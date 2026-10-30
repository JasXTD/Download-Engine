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
