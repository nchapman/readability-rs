"""Tests for default_parser_config()."""

from readability_uniffi import default_parser_config


def test_default_config_values():
    # Defaults from libreadability::Parser::new() — update if core defaults change
    config = default_parser_config()
    assert config.max_elems_to_parse == 0
    assert config.n_top_candidates == 5
    assert config.char_threshold == 500
    assert config.classes_to_preserve == ["page"]
    assert config.keep_classes is False
    assert "p" in config.tags_to_score
    assert "section" in config.tags_to_score
    assert config.disable_jsonld is False
