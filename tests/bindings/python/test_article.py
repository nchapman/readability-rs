"""Tests for Article fields and check_html()."""

from readability_uniffi import extract, check_html

ARTICLE_HTML = """
<html>
<head>
  <title>My Article Title</title>
  <meta name="author" content="Jane Doe">
  <meta property="og:description" content="A test article excerpt.">
</head>
<body>
  <article>
    <h1>My Article Title</h1>
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
</body>
</html>
"""


def test_article_fields():
    article = extract(ARTICLE_HTML, None)
    assert article.title == "My Article Title"
    assert len(article.content) > 0  # cleaned HTML
    assert len(article.text_content) > 0  # plain text
    assert article.length > 0
    assert article.length == len(article.text_content)
    # String fields should be strings (may be empty)
    assert isinstance(article.byline, str)
    assert isinstance(article.excerpt, str)
    assert isinstance(article.site_name, str)
    assert isinstance(article.image, str)
    assert isinstance(article.favicon, str)
    assert isinstance(article.language, str)
    assert isinstance(article.published_time, str)
    assert isinstance(article.modified_time, str)
    assert isinstance(article.dir, str)


def test_check_html_readable():
    assert check_html(ARTICLE_HTML) is True


def test_check_html_not_readable():
    assert check_html("<html><body><p>Short</p></body></html>") is False
