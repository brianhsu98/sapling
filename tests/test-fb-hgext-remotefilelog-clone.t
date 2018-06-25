
  $ . "$TESTDIR/library.sh"

  $ hginit master
  $ cd master
  $ cat >> .hg/hgrc <<EOF
  > [remotefilelog]
  > server=True
  > EOF
  $ echo x > x
  $ hg commit -qAm x

  $ cd ..

# shallow clone from full

  $ hgcloneshallow ssh://user@dummy/master shallow --noupdate
  streaming all changes
  2 files to transfer, 227 bytes of data
  transferred 227 bytes in * seconds (*/sec) (glob)
  searching for changes
  no changes found
  $ cd shallow
  $ cat .hg/requires
  dotencode
  fncache
  generaldelta
  remotefilelog
  revlogv1
  store
  treedirstate

  $ hg update
  1 files updated, 0 files merged, 0 files removed, 0 files unresolved
  1 files fetched over 1 fetches - (1 misses, 0.00% hit ratio) over *s (glob)

  $ cat x
  x

  $ ls .hg/store/data
  $ echo foo > f
  $ hg add f
  $ hg ci -m 'local content'
  $ ls .hg/store/data
  4a0a19218e082a343a1b17e5333409af9d98f0f5

  $ cd ..

# shallow clone from shallow

  $ hgcloneshallow ssh://user@dummy/shallow shallow2  --noupdate
  streaming all changes
  3 files to transfer, 564 bytes of data
  transferred 564 bytes in * seconds (*/sec) (glob)
  searching for changes
  no changes found
  $ cd shallow2
  $ cat .hg/requires
  dotencode
  fncache
  generaldelta
  remotefilelog
  revlogv1
  store
  treedirstate
  $ ls .hg/store/data
  4a0a19218e082a343a1b17e5333409af9d98f0f5

  $ hg update
  2 files updated, 0 files merged, 0 files removed, 0 files unresolved

  $ cat x
  x

  $ cd ..

# full clone from shallow

Note: the output to STDERR comes from a different process to the output on
STDOUT and their relative ordering is not deterministic. As a result, the test
was failing sporadically. To avoid this, we capture STDERR to a file and
check its contents separately.

  $ TEMP_STDERR=full-clone-from-shallow.stderr.tmp
  $ hg clone --noupdate ssh://user@dummy/shallow full 2>$TEMP_STDERR
  streaming all changes
  [255]
  $ cat $TEMP_STDERR
  remote: abort: Cannot clone from a shallow repo to a full repo.
  abort: unexpected response from remote server: empty string
  $ rm $TEMP_STDERR

# getbundle full clone

  $ printf '[server]\npreferuncompressed=False\n' >> master/.hg/hgrc
  $ hgcloneshallow ssh://user@dummy/master shallow3
  requesting all changes
  adding changesets
  adding manifests
  adding file changes
  added 1 changesets with 0 changes to 0 files
  new changesets b292c1e3311f
  updating to branch default
  1 files updated, 0 files merged, 0 files removed, 0 files unresolved

  $ ls shallow3/.hg/store/data
  $ cat shallow3/.hg/requires
  dotencode
  fncache
  generaldelta
  remotefilelog
  revlogv1
  store
  treedirstate
