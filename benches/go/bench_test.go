// Go benchmarks for go-readability, mirroring benches/extraction.rs.
//
// Run with:
//   go test -bench=. -benchmem -benchtime=5s
//
// Or via the comparison script:
//   ../../benches/compare.sh

package bench

import (
	"net/url"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	readability "codeberg.org/readeck/go-readability/v2"
)

var pageURL, _ = url.Parse("http://fakehost/test/page.html")

// testPagesDir returns the absolute path to the test-pages directory.
func testPagesDir() string {
	_, file, _, _ := runtime.Caller(0)
	// file is .../benches/go/bench_test.go → go up two dirs to repo root
	root := filepath.Join(filepath.Dir(file), "..", "..")
	return filepath.Join(root, "test-pages")
}

// loadFixture reads source.html for the named fixture.
func loadFixture(name string) string {
	path := filepath.Join(testPagesDir(), name, "source.html")
	data, err := os.ReadFile(path)
	if err != nil {
		panic("failed to read fixture " + name + ": " + err.Error())
	}
	return string(data)
}

// ── individual page benchmarks ────────────────────────────────────────────────

func benchPage(b *testing.B, name string) {
	b.Helper()
	html := loadFixture(name)
	b.SetBytes(int64(len(html)))
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		p := readability.NewParser()
		if _, err := p.Parse(strings.NewReader(html), pageURL); err != nil {
			b.Fatal(err)
		}
	}
}

func BenchmarkArs1(b *testing.B)     { benchPage(b, "ars-1") }     // ~56 KB
func BenchmarkWapo1(b *testing.B)    { benchPage(b, "wapo-1") }    // ~180 KB
func BenchmarkWikipedia(b *testing.B) { benchPage(b, "wikipedia") } // ~244 KB
func BenchmarkNytimes3(b *testing.B) { benchPage(b, "nytimes-3") } // ~489 KB
func BenchmarkYahoo2(b *testing.B)   { benchPage(b, "yahoo-2") }   // ~1.6 MB

// ── full fixture suite throughput ─────────────────────────────────────────────

func BenchmarkAllFixtures(b *testing.B) {
	entries, err := os.ReadDir(testPagesDir())
	if err != nil {
		b.Fatal(err)
	}

	type page struct{ html string }
	var pages []page
	var totalBytes int64

	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		path := filepath.Join(testPagesDir(), e.Name(), "source.html")
		data, err := os.ReadFile(path)
		if err != nil {
			continue
		}
		pages = append(pages, page{string(data)})
		totalBytes += int64(len(data))
	}

	b.SetBytes(totalBytes)
	b.ResetTimer()

	for i := 0; i < b.N; i++ {
		for _, pg := range pages {
			p := readability.NewParser()
			_, _ = p.Parse(strings.NewReader(pg.html), pageURL)
		}
	}
}
