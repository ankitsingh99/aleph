# Homebrew tap template for aleph. Don't edit the rendered copy by hand:
#   packaging/homebrew/render.sh --version 1.0.0 --repo owner/aleph dist/SHA256SUMS > aleph.rb
# then commit it as Formula/aleph.rb in your tap (github.com/owner/homebrew-tap).
# The release workflow attaches a rendered aleph.rb to every GitHub release.
class Aleph < Formula
  desc "One dashboard and CLI for coding agents running in herdr across your machines"
  homepage "https://github.com/@REPO@"
  version "@VERSION@"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/@REPO@/releases/download/v@VERSION@/aleph-aarch64-apple-darwin.tar.gz"
      sha256 "@SHA256_AARCH64_APPLE_DARWIN@"
    end
    on_intel do
      url "https://github.com/@REPO@/releases/download/v@VERSION@/aleph-x86_64-apple-darwin.tar.gz"
      sha256 "@SHA256_X86_64_APPLE_DARWIN@"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/@REPO@/releases/download/v@VERSION@/aleph-aarch64-unknown-linux-musl.tar.gz"
      sha256 "@SHA256_AARCH64_UNKNOWN_LINUX_MUSL@"
    end
    on_intel do
      url "https://github.com/@REPO@/releases/download/v@VERSION@/aleph-x86_64-unknown-linux-musl.tar.gz"
      sha256 "@SHA256_X86_64_UNKNOWN_LINUX_MUSL@"
    end
  end

  def install
    bin.install "aleph"
  end

  def caveats
    <<~EOS
      Add the short `al` command to ~/.zshrc:

        al() { "#{opt_bin}/aleph" "$@"; }

      Optional: route `claude` through Aleph too (Claude Code's own flags
      and subcommands still pass straight through to it):

        claude() { ALEPH_WRAP=1 "#{opt_bin}/aleph" "$@"; }

      Then pick a default machine and check the setup:

        aleph host pick
        aleph doctor

      herdr (https://herdr.dev) must be installed locally and on each machine.
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/aleph version </dev/null")
  end
end
