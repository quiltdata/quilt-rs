"""Smoke tests of the built module: it imports, and its calls reach the
engine. The engine's behaviour is tested in Rust (src/engine/tests.rs)."""

import json

import pytest
import quilt


def test_version_is_3():
    assert quilt.__version__.startswith("3.")


def test_a_domain_that_is_not_there_holds_nothing(tmp_path):
    domain = tmp_path / "domain"
    assert quilt.list_registries(domain=domain) == []
    assert quilt.list_stored(domain=domain) == []
    assert quilt.default_dataspace(domain=domain) is None
    assert not domain.exists()


def test_the_default_dataspace_comes_from_domain_json(tmp_path):
    domain = tmp_path / "domain"
    domain.mkdir()
    home = tmp_path / "home"
    (domain / "domain.json").write_text(
        json.dumps({"format": 2, "default_dataspace": str(home)})
    )
    assert quilt.default_dataspace(domain=domain) == home


def test_no_dataspace_and_no_default_raises(tmp_path):
    with pytest.raises(quilt.QuiltError, match="no default dataspace"):
        quilt.install(
            "quilt+s3://b#package=team/raw",
            domain=tmp_path / "domain",
            home=tmp_path / "home",
        )


def test_an_unknown_access_raises_and_writes_nothing(tmp_path):
    space = tmp_path / "space"
    space.mkdir()
    (space / "data.toml").write_text('registry = "s3://b"\n')
    with pytest.raises(quilt.QuiltError, match="not an access"):
        quilt.install(
            "quilt+s3://b#package=team/raw",
            dataspace=space,
            access="pull",
            domain=tmp_path / "domain",
            home=tmp_path / "home",
        )
    assert (space / "data.toml").read_text() == 'registry = "s3://b"\n'


def test_a_commit_of_a_package_not_declared_raises(tmp_path):
    space = tmp_path / "space"
    space.mkdir()
    (space / "data.toml").write_text(
        'registry = "s3://b"\n[packages]\nsync = ["team/raw"]\n'
    )
    with pytest.raises(quilt.QuiltError, match="not a package"):
        quilt.commit(
            "team/other",
            "first",
            dataspace=space,
            domain=tmp_path / "domain",
            home=tmp_path / "home",
        )
