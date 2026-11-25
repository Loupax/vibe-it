#!/bin/sh
# Writes a fontconfig file that exposes ONLY the Liberation fonts (the CI font
# set: apt package fonts-liberation) and maps the generics to them, so the test
# suite sees the same fonts on every machine (R33). Prints the file's path.
# Usage (run from scratchy/):
#   FONTCONFIG_FILE=$(tools/ci-fontconfig.sh LIBERATION_TTF_DIR WORKDIR) cargo test --release
# CI: LIBERATION_TTF_DIR=/usr/share/fonts/truetype/liberation
fonts="${1:?dir with the Liberation .ttf files}"
work="${2:?scratch dir for the config and cache}"
mkdir -p "$work/cache"
conf="$work/fonts.conf"
fam() { # generic, family
  printf '<match target="pattern"><test name="family" qual="any"><string>%s</string></test><edit name="family" mode="prepend" binding="strong"><string>%s</string></edit></match>\n' "$1" "$2"
}
{
  echo '<?xml version="1.0"?><!DOCTYPE fontconfig SYSTEM "fonts.dtd"><fontconfig>'
  echo "<dir>$fonts</dir><cachedir>$work/cache</cachedir>"
  # generic-family plumbing only (no font dirs); without it the rules below
  # do not take effect
  for f in 45-generic 45-latin 49-sansserif 60-generic 60-latin; do
    echo "<include ignore_missing=\"yes\">/etc/fonts/conf.d/$f.conf</include>"
  done
  fam sans-serif "Liberation Sans"
  fam serif "Liberation Serif"
  fam monospace "Liberation Mono"
  echo '</fontconfig>'
} > "$conf"
echo "$conf"
