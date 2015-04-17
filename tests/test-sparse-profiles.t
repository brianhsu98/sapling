test sparse

  $ hg init myrepo
  $ cd myrepo
  $ cat > .hg/hgrc <<EOF
  > [extensions]
  > sparse=$(dirname $TESTDIR)/sparse.py
  > purge=
  > strip=
  > rebase=
  > EOF

  $ echo a > index.html
  $ echo x > data.py
  $ echo z > readme.txt
  $ cat > webpage.sparse <<EOF
  > [include]
  > *.html
  > EOF
  $ cat > backend.sparse <<EOF
  > [include]
  > *.py
  > EOF
  $ hg ci -Aqm 'initial'

  $ hg sparse --include '*.sparse'

Verify enabling a single profile works

  $ hg sparse --enable-profile webpage.sparse
  $ ls
  backend.sparse
  index.html
  webpage.sparse

Verify enabling two profiles works

  $ hg sparse --enable-profile backend.sparse
  $ ls
  backend.sparse
  data.py
  index.html
  webpage.sparse

Verify disabling a profile works

  $ hg sparse --disable-profile webpage.sparse
  $ ls
  backend.sparse
  data.py
  webpage.sparse


Verify that a profile is updated across multiple commits

  $ cat > webpage.sparse <<EOF
  > [include]
  > *.html
  > EOF
  $ cat > backend.sparse <<EOF
  > [include]
  > *.py
  > *.txt
  > EOF

  $ echo foo >> data.py

  $ hg ci -m 'edit profile'
  $ ls
  backend.sparse
  data.py
  readme.txt
  webpage.sparse

  $ hg up -q 0
  $ ls
  backend.sparse
  data.py
  webpage.sparse

  $ hg up -q 1
  $ ls
  backend.sparse
  data.py
  readme.txt
  webpage.sparse

Introduce a conflicting .hgsparse change

  $ hg up -q 0
  $ cat > backend.sparse <<EOF
  > [include]
  > *.html
  > EOF
  $ echo bar >> data.py

  $ hg ci -qAm "edit profile other"
  $ ls
  backend.sparse
  index.html
  webpage.sparse

Verify conflicting merge pulls in the conflicting changes

  $ hg merge 1
  temporarily included 1 file(s) in the sparse checkout for merging
  merging backend.sparse
  warning: conflicts during merge.
  merging backend.sparse incomplete! (edit conflicts, then use 'hg resolve --mark')
  merging data.py
  warning: conflicts during merge.
  merging data.py incomplete! (edit conflicts, then use 'hg resolve --mark')
  0 files updated, 0 files merged, 0 files removed, 2 files unresolved
  use 'hg resolve' to retry unresolved file merges or 'hg update -C .' to abandon
  [1]

  $ rm *.orig
  $ ls
  backend.sparse
  data.py
  index.html
  webpage.sparse

Verify resolving the merge removes the temporarily unioned files

  $ cat > backend.sparse <<EOF
  > [include]
  > *.html
  > *.txt
  > EOF
  $ hg resolve -m backend.sparse

  $ cat > data.py <<EOF
  > x
  > foo
  > bar
  > EOF
  $ hg resolve -m data.py
  (no more unresolved files)

  $ hg ci -qAm "merge profiles"
  $ ls
  backend.sparse
  index.html
  readme.txt
  webpage.sparse

  $ hg cat -r . data.py
  x
  foo
  bar

Verify stripping refreshes dirstate

  $ hg strip -q -r .
  $ ls
  backend.sparse
  index.html
  webpage.sparse

Verify rebase conflicts pulls in the conflicting changes

  $ hg up -q 1
  $ ls
  backend.sparse
  data.py
  readme.txt
  webpage.sparse

  $ hg rebase -d 2
  rebasing 1:ac501d43ae16 "edit profile"
  temporarily included 1 file(s) in the sparse checkout for merging
  merging backend.sparse
  warning: conflicts during merge.
  merging backend.sparse incomplete! (edit conflicts, then use 'hg resolve --mark')
  merging data.py
  warning: conflicts during merge.
  merging data.py incomplete! (edit conflicts, then use 'hg resolve --mark')
  unresolved conflicts (see hg resolve, then hg rebase --continue)
  [1]
  $ rm *.orig
  $ ls
  backend.sparse
  data.py
  index.html
  webpage.sparse

Verify resolving conflict removes the temporary files

  $ cat > backend.sparse <<EOF
  > [include]
  > *.html
  > *.txt
  > EOF
  $ hg resolve -m backend.sparse

  $ cat > data.py <<EOF
  > x
  > foo
  > bar
  > EOF
  $ hg resolve -m data.py
  (no more unresolved files)

  $ hg rebase -q --continue
  $ ls
  backend.sparse
  index.html
  readme.txt
  webpage.sparse

  $ hg cat -r . data.py
  x
  foo
  bar

Test checking out a commit that does not contain the sparse profile

  $ hg sparse --reset
  $ hg rm *.sparse
  $ hg commit -m "delete profiles"
  $ hg up -q .^
  $ hg sparse --enable-profile backend.sparse
  $ ls
  index.html
  readme.txt
  $ hg up tip
  warning: sparse profile 'backend.sparse' not found in rev bc6a201ecffe - ignoring it
  1 files updated, 0 files merged, 2 files removed, 0 files unresolved
  $ ls
  data.py
  index.html
  readme.txt
  $ hg sparse --disable-profile backend.sparse
  warning: sparse profile 'backend.sparse' not found in rev bc6a201ecffe - ignoring it
