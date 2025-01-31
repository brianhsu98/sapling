# Copyright (c) Meta Platforms, Inc. and affiliates.
#
# This software may be used and distributed according to the terms of the
# GNU General Public License found in the LICENSE file in the root
# directory of this source tree.

  $ . "${TEST_FIXTURES}/library.sh"
  $ . "${TEST_FIXTURES}/library-xrepo-sync-with-git-submodules.sh"



Setup configuration
  $ run_common_xrepo_sync_with_gitsubmodules_setup

# This tests the scenario where a commit contains ONLY changes to git submodules
# i.e. there are not file changes that should be synced to the large repo.
# TODO(T169315758): Handle commits changes only to git submodules
Create commit that modifies git submodule in small repo
  $ testtool_drawdag -R "$SMALL_REPO_NAME" --no-default-files <<EOF
  > A-B-C
  > # modify: A "foo/a.txt" "creating foo directory"
  > # modify: A "bar/b.txt" "creating bar directory"
  > # modify: B "foo/git_submodule" git-submodule "creating git submodule"
  > # copy: C "foo/b.txt" "copying file from bar into foo" B "bar/b.txt"
  > # bookmark: C master
  > EOF
  A=7e97054c51a17ea2c03cd5184826b6a7556d141d57c5a1641bbd62c0854d1a36
  B=b51882d566acc1f3979a389e452e2c11ccdd05be65bf777c05924fc412b2cc71
  C=6473a332b6f2c52543365108144f9b1cff6b4874bc3ade72a8268f50226f86f4

  $ with_stripped_logs mononoke_x_repo_sync "$SMALL_REPO_ID"  "$LARGE_REPO_ID" initial-import --commit "$C" --version-name "$LATEST_CONFIG_VERSION_NAME" --new-bookmark "$NEW_BOOKMARK_NAME"
  using repo "small_repo" repoid RepositoryId(1)
  using repo "large_repo" repoid RepositoryId(0)
  using repo "small_repo" repoid RepositoryId(1)
  using repo "large_repo" repoid RepositoryId(0)
  changeset resolved as: ChangesetId(Blake2(6473a332b6f2c52543365108144f9b1cff6b4874bc3ade72a8268f50226f86f4))
  Checking if 6473a332b6f2c52543365108144f9b1cff6b4874bc3ade72a8268f50226f86f4 is already synced 1->0
  syncing 6473a332b6f2c52543365108144f9b1cff6b4874bc3ade72a8268f50226f86f4
  Setting bookmark SYNCED_HEAD to changeset 5cac851d3a164f682613d6901e17a03e18afe8576145d4f5ff9dd0a51a82437f
  changeset 6473a332b6f2c52543365108144f9b1cff6b4874bc3ade72a8268f50226f86f4 synced as 5cac851d3a164f682613d6901e17a03e18afe8576145d4f5ff9dd0a51a82437f in * (glob)
  successful sync



  $ clone_and_log_large_repo "$NEW_BOOKMARK_NAME" "$C"
  commit:      f9abb21ba833
  bookmark:    SYNCED_HEAD
  user:        author
  date:        Thu Jan 01 00:00:00 1970 +0000
  summary:     C
  
   smallrepofolder1/foo/b.txt |  1 +
   1 files changed, 1 insertions(+), 0 deletions(-)
  
  commit:      039696fd865f
  user:        author
  date:        Thu Jan 01 00:00:00 1970 +0000
  summary:     B
  
  
  commit:      e462fc947f26
  user:        author
  date:        Thu Jan 01 00:00:00 1970 +0000
  summary:     A
  
   smallrepofolder1/bar/b.txt |  1 +
   smallrepofolder1/foo/a.txt |  1 +
   2 files changed, 2 insertions(+), 0 deletions(-)
  
  
  
  Running mononoke_admin to verify mapping
  
  using repo "small_repo" repoid RepositoryId(1)
  using repo "large_repo" repoid RepositoryId(0)
  changeset resolved as: ChangesetId(Blake2(6473a332b6f2c52543365108144f9b1cff6b4874bc3ade72a8268f50226f86f4))
  RewrittenAs([(ChangesetId(Blake2(5cac851d3a164f682613d6901e17a03e18afe8576145d4f5ff9dd0a51a82437f)), CommitSyncConfigVersion("INITIAL_IMPORT_SYNC_CONFIG"))])
