# Zeno, for Homebrew. This file is written by tools/package.sh when a
# version is released: change that, not this.
#
#     brew tap partham20/zeno-lang https://github.com/partham20/zeno-lang
#     brew install partham20/zeno-lang/zeno
class Zeno < Formula
  desc "Small, readable, statically typed language for firmware"
  homepage "https://zeno-lang.org"
  version "0.3.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.3.0/zeno-0.3.0-macos-arm64.tar.gz"
      sha256 "8372a39dcd19b0955986e5be18a153018eda638235553bdefe988efd59a3931a"
    end
    on_intel do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.3.0/zeno-0.3.0-macos-x86_64.tar.gz"
      sha256 "bee1d9a2d8f09b4e7e75e3d954ba3be93c5495b64b24eda4445f456b01a35025"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.3.0/zeno-0.3.0-linux-arm64.tar.gz"
      sha256 "e2313a6238b2e8cc3b29429dbea895b1a2e01793aa0f492c6fbafc3f36edc8c9"
    end
    on_intel do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.3.0/zeno-0.3.0-linux-x86_64.tar.gz"
      sha256 "af69d47040527e7535d3579076c8e17d7122c88012f82f898704e9d7639939c9"
    end
  end

  def install
    bin.install "zeno"
    # Zeno built for a browser, which `zeno build --web` looks for
    # beside `zeno`.
    bin.install "zeno.wasm" if File.exist?("zeno.wasm")
    doc.install "README.md", "docs"
    pkgshare.install "examples", "editors"
  end

  def caveats
    <<~EOS
      `zeno build` hands the C it writes to a C compiler, and `zeno fetch`
      gets packages with git. On macOS both come with Apple's command
      line tools: xcode-select --install
      The examples are in #{opt_pkgshare}/examples
    EOS
  end

  test do
    assert_equal "zeno #{version}", shell_output("#{bin}/zeno --version").strip
    assert_equal "Hello, Zeno!", shell_output("#{bin}/zeno run #{pkgshare}/examples/hello.zeno").strip
  end
end
