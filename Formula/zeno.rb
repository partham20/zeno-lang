# Zeno, for Homebrew. This file is written by tools/package.sh when a
# version is released: change that, not this.
#
#     brew tap zeno-src/zeno-lang https://github.com/zeno-src/zeno-lang
#     brew install zeno-src/zeno-lang/zeno
class Zeno < Formula
  desc "Small, readable, statically typed language for firmware"
  homepage "https://zeno-lang.org"
  version "0.7.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.7.0/zeno-0.7.0-macos-arm64.tar.gz"
      sha256 "0fdbab63165c502492a036971bc799204aa25d8f190d10f442076a1039cd3573"
    end
    on_intel do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.7.0/zeno-0.7.0-macos-x86_64.tar.gz"
      sha256 "603a6f4a3df397f1cea9f8a6020fae8394921198e872294bb7425ba6ad4479cb"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.7.0/zeno-0.7.0-linux-arm64.tar.gz"
      sha256 "c6a82933ff41b622aaba12b2716e797b76ca72090ff0abd0dcf4a9a92889da70"
    end
    on_intel do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.7.0/zeno-0.7.0-linux-x86_64.tar.gz"
      sha256 "2717dde867e2c049edca569f79d4a6fd6ff62d81467337180e1935e6f755b4fe"
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
