# Homebrew formula for termboy.
#
# This lives here for version control; the live copy belongs in a tap repo named
# `homebrew-tap` under your account, so `brew install roma-888/tap/termboy` works:
#
#     gh repo create roma-888/homebrew-tap --public
#     # add this file as Formula/termboy.rb in that repo
#
# Before publishing a new version, bump `url` to the new tag and refresh sha256:
#     curl -sL https://github.com/roma-888/termboy/archive/refs/tags/v0.3.0.tar.gz | shasum -a 256
#
# This builds from source (a few minutes — fat LTO). Switch to bottling later if
# you want instant installs.
class Termboy < Formula
  desc "Game Boy, Game Boy Color, and Game Boy Advance emulator for your terminal"
  homepage "https://github.com/roma-888/termboy"
  url "https://github.com/roma-888/termboy/archive/refs/tags/v0.3.0.tar.gz"
  sha256 "a2506be0b58afa117c700d6a5168c7a86671f06c5be10045590e316869adbcaf"
  license "MIT"
  head "https://github.com/roma-888/termboy.git", branch: "main"

  depends_on "rust" => :build

  on_linux do
    depends_on "alsa-lib" # cpal links against ALSA
  end

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/termboy")
  end

  test do
    assert_path_exists bin/"termboy"
  end
end
