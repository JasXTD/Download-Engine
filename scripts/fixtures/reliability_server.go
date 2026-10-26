// Deterministic range server for Windows reliability tests. Standard library only.
// Run with a read-only root filesystem, no host data, and a loopback-only port.
package main

import (
	"encoding/binary"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"
)

const mib int64 = 1024 * 1024
const gib int64 = 1024 * mib

type request struct {
	Phase int32  `json:"phase"`
	Start int64  `json:"start"`
	End   int64  `json:"end"`
	Sent  int64  `json:"sent"`
	URL   string `json:"url"`
}

func main() {
	mode := os.Getenv("FIXTURE_MODE")
	size := 16*mib + 17
	if mode == "large" {
		size = 4*gib + 64*mib + 17
	}
	var phase atomic.Int32
	var mu sync.Mutex
	records := make([]*request, 0)
	pattern := make([]byte, mib)
	for i := range pattern {
		pattern[i] = byte((i*31 + i/251) % 251)
	}
	http.HandleFunc("/health", func(w http.ResponseWriter, r *http.Request) {
		json.NewEncoder(w).Encode(map[string]any{"mode": mode, "size": size})
	})
	http.HandleFunc("/phase", func(w http.ResponseWriter, r *http.Request) {
		n, err := strconv.Atoi(r.URL.Query().Get("value"))
		if r.Method != "POST" || err != nil || n < 0 || n > 2 {
			http.Error(w, "Invalid phase", 400)
			return
		}
		phase.Store(int32(n))
		w.WriteHeader(204)
	})
	http.HandleFunc("/requests", func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		json.NewEncoder(w).Encode(records)
	})
	http.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		start, end := int64(0), size-1
		ranged := r.Header.Get("Range") != ""
		if ranged {
			parts := strings.Split(strings.TrimPrefix(r.Header.Get("Range"), "bytes="), "-")
			var err error
			if len(parts) != 2 {
				http.Error(w, "Invalid range", 416)
				return
			}
			start, err = strconv.ParseInt(parts[0], 10, 64)
			if err != nil {
				http.Error(w, "Invalid range", 416)
				return
			}
			if parts[1] != "" {
				end, err = strconv.ParseInt(parts[1], 10, 64)
				if err != nil {
					http.Error(w, "Invalid end", 416)
					return
				}
			}
		}
		if start < 0 || end < start || end >= size {
			http.Error(w, "Invalid range", 416)
			return
		}
		record := &request{Phase: phase.Load(), Start: start, End: end, URL: r.URL.Path}
		mu.Lock()
		records = append(records, record)
		mu.Unlock()
		w.Header().Set("Content-Length", strconv.FormatInt(end-start+1, 10))
		w.Header().Set("Accept-Ranges", "bytes")
		w.Header().Set("ETag", `"reliability-v1"`)
		w.Header().Set("Content-Type", "application/octet-stream")
		if ranged {
			w.Header().Set("Content-Range", fmt.Sprintf("bytes %d-%d/%d", start, end, size))
			w.WriteHeader(206)
		}
		if r.Method == "HEAD" {
			return
		}
		buffer := make([]byte, mib)
		for at := start; at <= end; {
			gate := size
			if mode == "large" {
				if phase.Load() == 0 {
					gate = 32 * mib
				} else if phase.Load() == 1 {
					gate = 4*gib + 8*mib
				}
			} else if phase.Load() == 0 {
				gate = mib
			}
			if at >= gate {
				select {
				case <-r.Context().Done():
					return
				case <-time.After(20 * time.Millisecond):
				}
				continue
			}
			base := at / mib * mib
			copy(buffer, pattern)
			for i := int64(0); i < mib; i += 4096 {
				binary.LittleEndian.PutUint64(buffer[i:], uint64(base+i))
			}
			offset := at % mib
			count := min(mib-offset, end-at+1, gate-at)
			n, err := w.Write(buffer[offset : offset+count])
			mu.Lock()
			record.Sent += int64(n)
			mu.Unlock()
			if err != nil {
				return
			}
			at += int64(n)
		}
	})
	server := &http.Server{Addr: ":8080", ReadHeaderTimeout: 5 * time.Second, IdleTimeout: 10 * time.Second}
	fmt.Println("reliability fixture ready")
	if err := server.ListenAndServe(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
