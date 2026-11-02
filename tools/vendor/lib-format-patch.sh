# shellcheck shell=bash
# Sourced by export-patches.sh and export-c-patches.sh (not run on its own,
# hence no shebang; the directive above tells shellcheck the dialect).
# Defines format_patch_pinned, the one place that says how a vendor patch is
# written, so the two exporters cannot drift apart.
#
# The committed patch files are compared byte-for-byte (a re-export of an
# unchanged stack must produce no diff), but `git format-patch` output depends
# on the caller's configuration: format.{thread,subjectPrefix,coverLetter,
# headers,from,signOff,attach,signature,...} change the headers and framing;
# diff.{noprefix,mnemonicPrefix,renames,algorithm,context,interHunkContext,
# indentHeuristic,suppressBlankEmpty} and core.abbrev change the diff body and
# the `index` lines; diff.orderFile reorders the files within a patch, and a
# user-level core.attributesFile (git reads ~/.config/git/attributes even when
# the key is unset) can assign a `diff=` driver whose funcname pattern changes
# the hunk headers; format.filenameMaxLength changes the file names, which
# carry the series order.
# Each is pinned here to a fixed value. That value is git's own default for
# every setting but four: core.abbrev (the seven characters the committed
# series uses, which also stops the index lines changing when the repository's
# object count grows); --no-encode-email-headers (git encodes non-ASCII
# headers by default, the committed series is raw); and the orderfile and
# attributes file, whose default is "whatever the user has", pinned to
# /dev/null (an empty *value* is fatal on git 2.56: "failed to read orderfile").
# Flags are used where git has one, since they also override the repository's
# own config; `-c` only for settings that have none. The environment is
# deliberately not isolated with GIT_CONFIG_GLOBAL, which would also drop
# safe.directory.
#
# Residuals, deliberately not pinned (a decision recorded in internal #103):
#   - core.quotePath: changes how non-ASCII file names are quoted.
#   - i18n.logOutputEncoding, i18n.commitEncoding: matter only for commit
#     messages that are not UTF-8.
#   - export-c-patches.sh does not pass --numbered, so a one-patch C range
#     writes "[PATCH]" where export-patches.sh writes "[PATCH 1/1]".
#   - export-c-patches.sh's pinned format-patch output has no byte-identity
#     test, because its C series is empty (test-check-patches.sh checks the
#     pinned output through export-patches.sh only).
#
# --zero-commit and --no-signature keep the files stable across rebases and git
# versions. Callers add --numbered (or not) and the range themselves.
#
# Two pairs pin the same output twice, as intentional defence in depth, so
# removing one member of a pair alone changes nothing (the tests in
# test-check-patches.sh section 2c check each pair as a whole):
#   - the a/ b/ prefixes: -c diff.noprefix/diff.mnemonicPrefix, and
#     --src-prefix/--dst-prefix. On git 2.56 neither setting moves
#     format-patch off a/ b/ (it stopped honouring diff.noprefix in 2.41);
#     they stay for older git. The flags also override diff.srcPrefix and
#     diff.dstPrefix (git 2.45+), which 2.56's format-patch does honour.
#   - the recipients: --no-to/--no-cc, and --no-add-header, which also clears
#     the To: and Cc: lists that format.to/format.cc fill.
# --quiet only stops the file names being listed on stdout; it does not change
# the bytes written, so no test covers it.
# Usage: format_patch_pinned [format-patch args...]
format_patch_pinned() {
  git -c core.abbrev=7 -c diff.noprefix=false -c diff.mnemonicPrefix=false \
    -c diff.renames=true -c diff.algorithm=myers -c diff.context=3 \
    -c diff.interHunkContext=0 -c diff.indentHeuristic=true \
    -c diff.suppressBlankEmpty=false -c core.attributesFile=/dev/null \
    format-patch --quiet --zero-commit --no-signature \
    --no-thread --no-cover-letter --subject-prefix=PATCH --no-from \
    --no-signoff --no-attach --no-add-header --no-to --no-cc --no-base \
    --no-notes --no-encode-email-headers --suffix=.patch \
    --filename-max-length=64 \
    --src-prefix=a/ --dst-prefix=b/ -O/dev/null "$@"
}
