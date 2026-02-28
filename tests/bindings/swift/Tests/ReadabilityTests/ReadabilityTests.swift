import XCTest
import Readability

final class ReadabilityTests: XCTestCase {

    private let articleHtml = """
        <html>
        <head><title>Test Article</title></head>
        <body>
          <nav><a href="/">Home</a> | <a href="/about">About</a></nav>
          <article>
            <h1>Test Article</h1>
            <p>This is the first paragraph of the article. It contains enough text to \
        exceed the character threshold that the readability algorithm uses to determine \
        whether content is substantial enough to be considered an article.</p>
            <p>The second paragraph adds more content to ensure the algorithm can properly \
        identify this as the main content of the page. Readability works by scoring \
        DOM nodes based on their text density, paragraph count, and other heuristics.</p>
            <p>A third paragraph further strengthens the signal that this is genuine article \
        content rather than boilerplate navigation or sidebar material. The algorithm \
        compares candidate nodes and selects the one with the highest score.</p>
            <p>Finally, a fourth paragraph ensures we are well above the default character \
        threshold of five hundred characters, making this a reliable test fixture for \
        the extraction algorithm across all three language bindings.</p>
          </article>
          <footer>Copyright 2024</footer>
        </body>
        </html>
        """

    // MARK: - extract()

    func testExtractBasic() throws {
        let article = try extract(html: articleHtml, url: nil)
        XCTAssertTrue(article.textContent.contains("first paragraph"))
        XCTAssertEqual(article.title, "Test Article")
    }

    func testExtractEmptyBody() throws {
        let article = try extract(html: "<html><body></body></html>", url: nil)
        XCTAssertEqual(article.textContent, "")
        XCTAssertEqual(article.length, 0)
    }

    func testExtractWithUrl() throws {
        let article = try extract(html: articleHtml, url: "https://example.com/article")
        XCTAssertTrue(article.textContent.contains("first paragraph"))
    }

    func testExtractWithInvalidUrl() {
        XCTAssertThrowsError(try extract(html: articleHtml, url: "not a url")) { error in
            guard case ReadabilityError.Parse = error else {
                XCTFail("Expected ReadabilityError.Parse, got \(error)")
                return
            }
        }
    }

    // MARK: - extract_with()

    func testExtractWithDefaultConfig() throws {
        let a = try extract(html: articleHtml, url: nil)
        let b = try extractWith(html: articleHtml, url: nil, config: defaultParserConfig())
        XCTAssertEqual(a.textContent, b.textContent)
    }

    func testExtractWithCustomConfig() throws {
        var config = defaultParserConfig()
        config.charThreshold = 10
        let article = try extractWith(html: articleHtml, url: nil, config: config)
        XCTAssertTrue(article.textContent.count > 0)
    }

    // MARK: - Article fields

    func testArticleFields() throws {
        let article = try extract(html: articleHtml, url: nil)
        XCTAssertEqual(article.title, "Test Article")
        XCTAssertTrue(article.content.count > 0)
        XCTAssertTrue(article.textContent.count > 0)
        XCTAssertTrue(article.length > 0)
        // Note: article.length is Rust str::len() (byte count), not Swift character count
    }

    // MARK: - check_html()

    func testCheckHtmlReadable() {
        XCTAssertTrue(checkHtml(html: articleHtml))
    }

    func testCheckHtmlNotReadable() {
        XCTAssertFalse(checkHtml(html: "<html><body><p>Short</p></body></html>"))
    }

    // MARK: - default_parser_config()

    func testDefaultConfig() {
        let config = defaultParserConfig()
        XCTAssertEqual(config.maxElemsToParse, 0)
        XCTAssertEqual(config.nTopCandidates, 5)
        XCTAssertEqual(config.charThreshold, 500)
        XCTAssertEqual(config.classesToPreserve, ["page"])
        XCTAssertFalse(config.keepClasses)
        XCTAssertTrue(config.tagsToScore.contains("p"))
        XCTAssertTrue(config.tagsToScore.contains("section"))
        XCTAssertFalse(config.disableJsonld)
    }
}
