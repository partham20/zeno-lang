# Zeno, for Homebrew. This file is written by tools/package.sh when a
# version is released: change that, not this.
#
#     brew tap zeno-src/zeno-lang https://github.com/zeno-src/zeno-lang
#     brew install zeno-src/zeno-lang/zeno
class Zeno < Formula
  desc "Small, readable, statically typed language for firmware"
  homepage "https://zeno-lang.org"
  version "0.5.1"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.5.1/zeno-0.5.1-macos-arm64.tar.gz"
      sha256 "1f6663e858461372dc116e1dd27c9c7f7e0282ea4a12749050d53c9f9c42de6a"
    end
    on_intel do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.5.1/zeno-0.5.1-macos-x86_64.tar.gz"
      sha256 "ea29745767ecdf1f959aaffe204f99d1fec779fa3af8155decfb09dd614ee5c8"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.5.1/zeno-0.5.1-linux-arm64.tar.gz"
      sha256 "cb69d0c759f5bf437480e728e63092cea46b4528bae10d8164d484598f59256b"
    end
    on_intel do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.5.1/zeno-0.5.1-linux-x86_64.tar.gz"
      sha256 "318e58181522b50eaf2b51aea7083bac5d0dd320e05f1a502355643d5b9a6bcd"
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
