"""Tests for extract() and extract_with()."""

import pytest

from readability_uniffi import (
    ReadabilityError,
    extract,
    extract_with,
    default_parser_config,
)


ARTICLE_HTML = """
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
"""


def test_extract_basic():
    article = extract(ARTICLE_HTML, None)
    assert "first paragraph" in article.text_content
    assert article.title == "Test Article"


def test_extract_empty_body():
    # Empty body: library returns an empty article rather than raising an error
    article = extract("<html><body></body></html>", None)
    assert article.text_content == ""
    assert article.length == 0


def test_extract_short_content():
    # Short content: library extracts whatever it can, no error raised
    article = extract("<html><body><p>Short</p></body></html>", None)
    assert article.length <= 10


def test_extract_with_url():
    html = ARTICLE_HTML.replace('href="/', 'href="https://example.com/')
    article = extract(html, "https://example.com/article")
    assert "first paragraph" in article.text_content


def test_extract_with_invalid_url():
    with pytest.raises(ReadabilityError.Parse):
        extract(ARTICLE_HTML, "not a url")


def test_extract_with_default_config():
    article_a = extract(ARTICLE_HTML, None)
    article_b = extract_with(ARTICLE_HTML, None, default_parser_config())
    assert article_a.text_content == article_b.text_content


def test_extract_with_custom_config():
    """Lower char_threshold so even short content can be extracted."""
    config = default_parser_config()
    config.char_threshold = 10
    article = extract_with(ARTICLE_HTML, None, config)
    assert len(article.text_content) > 0
