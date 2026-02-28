import uniffi.readability_uniffi.*
import kotlin.test.*
import org.junit.jupiter.api.Nested

class ReadabilityTest {

    private val articleHtml = """
        <html>
        <head><title>Test Article</title></head>
        <body>
          <nav><a href="/">Home</a> | <a href="/about">About</a></nav>
          <article>
            <h1>Test Article</h1>
            <p>This is the first paragraph of the article. It contains enough text to
            exceed the character threshold that the readability algorithm uses to determine
            whether content is substantial enough to be considered an article.</p>
            <p>The second paragraph adds more content to ensure the algorithm can properly
            identify this as the main content of the page. Readability works by scoring
            DOM nodes based on their text density, paragraph count, and other heuristics.</p>
            <p>A third paragraph further strengthens the signal that this is genuine article
            content rather than boilerplate navigation or sidebar material. The algorithm
            compares candidate nodes and selects the one with the highest score.</p>
            <p>Finally, a fourth paragraph ensures we are well above the default character
            threshold of five hundred characters, making this a reliable test fixture for
            the extraction algorithm across all three language bindings.</p>
          </article>
          <footer>Copyright 2024</footer>
        </body>
        </html>
    """.trimIndent()

    @Nested inner class Extract {

        @Test fun `basic extraction`() {
            val article = extract(articleHtml, null)
            assertTrue(article.textContent.contains("first paragraph"))
            assertEquals("Test Article", article.title)
        }

        @Test fun `empty body returns empty article`() {
            val article = extract("<html><body></body></html>", null)
            assertEquals("", article.textContent)
            assertEquals(0L, article.length)
        }

        @Test fun `with url`() {
            val article = extract(articleHtml, "https://example.com/article")
            assertTrue(article.textContent.contains("first paragraph"))
        }

        @Test fun `invalid url throws Parse`() {
            assertFailsWith<ReadabilityException.Parse> {
                extract(articleHtml, "not a url")
            }
        }
    }

    @Nested inner class ExtractWith {

        @Test fun `default config matches extract`() {
            val a = extract(articleHtml, null)
            val b = extractWith(articleHtml, null, defaultParserConfig())
            assertEquals(a.textContent, b.textContent)
        }

        @Test fun `custom config`() {
            val config = defaultParserConfig().copy(charThreshold = 10L)
            val article = extractWith(articleHtml, null, config)
            assertTrue(article.textContent.isNotEmpty())
        }
    }

    @Nested inner class ArticleFields {

        @Test fun `all fields present`() {
            val article = extract(articleHtml, null)
            assertEquals("Test Article", article.title)
            assertTrue(article.content.isNotEmpty())
            assertTrue(article.textContent.isNotEmpty())
            assertTrue(article.length > 0L)
        }
    }

    @Nested inner class CheckHtml {

        @Test fun `readable document`() {
            assertTrue(checkHtml(articleHtml))
        }

        @Test fun `not readable document`() {
            assertFalse(checkHtml("<html><body><p>Short</p></body></html>"))
        }
    }

    @Nested inner class Config {

        @Test fun `default config values`() {
            val config = defaultParserConfig()
            assertEquals(0L, config.maxElemsToParse)
            assertEquals(5L, config.nTopCandidates)
            assertEquals(500L, config.charThreshold)
            assertEquals(listOf("page"), config.classesToPreserve)
            assertFalse(config.keepClasses)
            assertTrue(config.tagsToScore.contains("p"))
            assertTrue(config.tagsToScore.contains("section"))
            assertFalse(config.disableJsonld)
        }
    }
}
