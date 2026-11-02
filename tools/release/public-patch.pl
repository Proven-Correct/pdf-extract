#!/usr/bin/env perl
# Rewrite one committed vendor patch (vendor/patches/hayro/*.patch) into the
# form the public repository's history replays and regenerates (design D3 of
# publish-publicly-for-adoption; ADR 0097). build-public-history.sh feeds each
# patch through this filter before `git am`, and later compares the regenerated
# series against the same filter's output, so this file is the single
# definition of "the committed patch, made public".
#
# Usage: public-patch.pl 'Name <email>' < in.patch > out.patch
#
# Two changes, both confined to the mail header and the commit message (the
# text before the first line that is exactly "---"); the diff itself is never
# touched, since it must stay byte-identical for `git am` and for the vendor
# drift checks:
#   1. The header's "From:" line names the public identity. The leading mbox
#      line ("From 0000... Mon Sep 17 ...") is not a header and is kept.
#   2. Internal issue and pull-request references in the message body read
#      "internal #N", by the same rules the one-time tree rewrite used
#      (ADR 0097): the first reference of a run joined by ",", "and", "or",
#      "&", a dash, "/" or "+" is qualified and the rest of the run inherits
#      it; a preceding "PR", "issue" or "pull request" word takes the
#      qualifier instead ("internal PR #12"); a reference already preceded by
#      "internal" is left alone; HTML entities such as "&#12;" are not
#      references. Unlike the tree rewrite, which skipped numbers above 200 as
#      unlikely references, every number of up to four digits is qualified:
#      the internal tracker is already near 200, and a bare "Fixes #2NN" in a
#      public commit would close an unrelated public issue.
# A reference in the Subject is refused rather than qualified: the patch's
# file name is derived from its subject, so qualifying one would rename the
# file and break the series comparison in a way that reads as corruption.
#
# The file is processed as raw bytes (no decoding), so everything outside the
# two rewrites round-trips exactly whatever its encoding.
use strict;
use warnings;

die "usage: public-patch.pl 'Name <email>' < in.patch > out.patch\n"
  unless @ARGV == 1 && $ARGV[0] =~ /^[^<>\n]+ <[^<>\s]+>$/;
my $identity = $ARGV[0];

binmode STDIN;
binmode STDOUT;
my $patch = do { local $/; <STDIN> };

# Mail header: up to the first blank line. Message: from there to the first
# line "---", which format-patch puts before the diffstat; it may be empty (a
# commit with a subject and no body), so the message group is the blank line
# plus zero or more whole lines.
$patch =~ /\A(.*?\n)(\n(?:.*?\n)??)(---\n.*)\z/s
  or die "public-patch.pl: no mail header and '---' separator; not a format-patch file\n";
my ($header, $message, $rest) = ($1, $2, $3);

my $from_lines = () = $header =~ /^From: /mg;
die "public-patch.pl: expected exactly one From: header, found $from_lines\n"
  unless $from_lines == 1;
$header =~ s/^From: .*$/From: $identity/m;

# The Subject header, with its folded continuation lines.
if ($header =~ /^(Subject: .*(?:\n[ \t].*)*)/m) {
  my $subject = $1;
  die "public-patch.pl: internal reference in the Subject; rename the commit instead\n"
    if qualify($subject) ne $subject;
}

print $header, qualify($message), $rest;

# The ADR 0097 qualifier; see the header for the rules. Positions come from
# the token match on $t; the other matches run on copies, so they leave the
# /g position of $t alone.
sub qualify {
  my ($t) = @_;
  my ($out, $pos, $prev_end) = ('', 0, undef);
  while ($t =~ /(?<![\w\/&#{\$])(?<!\]\()#(\d{1,4})(?!\w)/g) {
    my ($start, $end) = ($-[0], $+[0]);
    my $gap = defined $prev_end ? substr($t, $prev_end, $start - $prev_end) : undef;
    my $continues = defined $gap
      # En and em dashes as their UTF-8 bytes, since $t is not decoded.
      && $gap =~ /^\s*(?:,|and|or|&|\xe2\x80\x93|\xe2\x80\x94|-|\/|\+|,\s*(?:and|or))\s*$/;
    $prev_end = $end;
    next if $continues;
    my $before = substr($t, 0, $start);
    my $anchor = $before =~ /\b(?:pull requests?|PRs?|issues?)\s+$/i ? $-[0] : $start;
    next if substr($t, 0, $anchor) =~ /(?<![\w-])internal\s+$/i;
    my $word = substr($t, $anchor, $start - $anchor);
    my $ins = 'internal ';
    if ($word =~ /^[A-Z]/ && substr($word, 0, 2) ne 'PR') {
      ($ins, $word) = ('Internal ', lcfirst $word);
    }
    $out .= substr($t, $pos, $anchor - $pos) . $ins . $word;
    $pos = $start;
  }
  return $out . substr($t, $pos);
}
