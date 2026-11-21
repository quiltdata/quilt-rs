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
    assert not domain.exists()


def test_the_default_dataspace_is_the_one_chosen_for_the_app(tmp_path):
    home = tmp_path / "home"
    assert quilt.default_dataspace(home=home) is None
    home.mkdir()
    chosen = tmp_path / "chosen"
    (home / "default_dataspace.json").write_text(json.dumps({"dataspace": str(chosen)}))
    assert quilt.default_dataspace(home=home) == chosen


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


def test_a_package_is_not_created_under_read(tmp_path):
    space = tmp_path / "space"
    space.mkdir()
    (space / "data.toml").write_text('registry = "s3://b"\n')
    with pytest.raises(quilt.QuiltError, match="no revision to `read`"):
        quilt.create(
            "quilt+s3://b#package=team/new",
            dataspace=space,
            access="read",
            domain=tmp_path / "domain",
            home=tmp_path / "home",
        )


def test_an_install_whose_read_fails_raises(tmp_path):
    """A failed read after declare must raise QuiltError for try/except."""
    space = tmp_path / "space"
    space.mkdir()
    (space / "data.toml").write_text(
        'registry = "s3://b"\n\n[hooks.pre-read]\ncommand = "false"\n'
    )
    # No remote package: the hook fails first when a package exists; without
    # one the install fails earlier. Either path must raise QuiltError, never
    # return a dict with outcome "failed".
    with pytest.raises(quilt.QuiltError):
        quilt.install(
            "quilt+s3://b#package=team/raw",
            dataspace=space,
            domain=tmp_path / "domain",
            home=tmp_path / "home",
        )


def test_a_local_package_is_created_with_no_warnings(tmp_path):
    """Warnings come back in the result, not as an exception; a package in
    the local registry has no workflows to warn of."""
    space = tmp_path / "space"
    space.mkdir()
    (space / "data.toml").write_text('registry = "s3://b"\n')
    folder = tmp_path / "analysis"
    folder.mkdir()
    (folder / "result.csv").write_text("1,2")
    created = quilt.create(
        "quilt+local://#package=me/analysis",
        dataspace=space,
        folder=folder,
        domain=tmp_path / "domain",
        home=tmp_path / "home",
    )
    assert isinstance(created["commit"], str)
    assert created["warnings"] == []


def test_deleting_a_package_no_store_holds_raises(tmp_path):
    domain = tmp_path / "domain"
    with pytest.raises(quilt.QuiltError, match="no local Package Store holds"):
        quilt.delete("team/raw", domain=domain)
    assert not domain.exists()


def test_a_short_prefix_raises(tmp_path):
    with pytest.raises(quilt.QuiltError, match="Invalid hash"):
        quilt.resolve_hash(
            "quilt+s3://b#package=team/raw", "12", domain=tmp_path / "domain"
        )


def test_browsing_a_package_never_read_raises(tmp_path):
    space = tmp_path / "space"
    space.mkdir()
    (space / "data.toml").write_text(
        'registry = "s3://b"\n[packages]\nsync = ["team/raw"]\n'
    )
    with pytest.raises(quilt.QuiltError, match="no revision in the lock"):
        quilt.browse(
            "team/raw",
            dataspace=space,
            domain=tmp_path / "domain",
            home=tmp_path / "home",
        )
