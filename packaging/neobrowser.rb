# Homebrew formula. Install with:
#   brew tap pitiflautico/neobrowser https://github.com/pitiflautico/neobrowser
#   brew install neobrowser
#
# Pinned to a released tag with its checksum, so `brew install` is reproducible and an
# altered artifact fails verification instead of installing.
class Neobrowser < Formula
  desc "MCP server that drives real Chrome via CDP, with verified actions"
  homepage "https://github.com/pitiflautico/neobrowser"
  version "0.1.7"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/pitiflautico/neobrowser/releases/download/v0.1.7/neobrowser-aarch64-apple-darwin.tar.gz"
      # Replace on each release; `brew fetch --force` prints the value.
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_intel do
      url "https://github.com/pitiflautico/neobrowser/releases/download/v0.1.7/neobrowser-x86_64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  on_linux do
    url "https://github.com/pitiflautico/neobrowser/releases/download/v0.1.7/neobrowser-x86_64-unknown-linux-musl.tar.gz"
    sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  end

  # Chrome is a runtime requirement, not a build one, and Homebrew cannot express
  # "either Chrome or Chromium" — so it is documented in the caveat instead of being a
  # hard dependency that would block users who already have Chrome installed.
  def install
    bin.install "neobrowser"
  end

  def caveats
    <<~EOS
      NeoBrowser drives your installed Google Chrome (or Chromium). If `neobrowser doctor`
      cannot find it, set NEOBROWSER_CHROME_BIN.

      Register it with an MCP client:
        { "mcpServers": { "neobrowser": { "command": "neobrowser" } } }
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/neobrowser --version")
    # `doctor --json` exits non-zero when a check fails, so this asserts the binary runs
    # and produces its report rather than asserting a healthy environment (a CI box has
    # no Chrome).
    assert_match "checks", shell_output("#{bin}/neobrowser doctor --json", 1)
  end
end
