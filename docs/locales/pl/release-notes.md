## 3.1.1 — 2026-10-27

### Szybsze kończenie dużych plików

Jednoczęściowe pobrania HTTP/HTTPS i FTP na tym samym dysku są teraz weryfikowane i zapisywane bez tworzenia drugiej pełnej kopii. Pliki wieloczęściowe zachowują dotychczasowe asynchroniczne scalanie we właściwej kolejności. Zapis na innym dysku nadal korzysta z bezpiecznego kopiowania. Zachowano weryfikację sum kontrolnych, anulowanie, ochronę istniejących nazw i oznaczenie pochodzenia z Internetu. Przerwany zapis jest odzyskiwany z zapisanej części lub pliku scalania bez ponownego pobierania i duplikatów.
