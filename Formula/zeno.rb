# Zeno, for Homebrew. This file is written by tools/package.sh when a
# version is released: change that, not this.
#
#     brew tap zeno-src/zeno-lang https://github.com/zeno-src/zeno-lang
#     brew install zeno-src/zeno-lang/zeno
class Zeno < Formula
  desc "Small, readable, statically typed language for firmware"
  homepage "https://zeno-lang.org"
  version "0.6.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.6.0/zeno-0.6.0-macos-arm64.tar.gz"
      sha256 "0d2228a59dedab7de4bb693d63029bf162ba00ec5bde7343fc30a2984243729d"
    end
    on_intel do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.6.0/zeno-0.6.0-macos-x86_64.tar.gz"
      sha256 "2146e586784c26b422777e86d5e3d4e5cfe7b30951d924bbe3d50760a00cd9b0"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.6.0/zeno-0.6.0-linux-arm64.tar.gz"
      sha256 "3ee3a119d8d2142cf42e1c09b2d37ac7d31013aaab83a3f8723ca62c29d09a51"
    end
    on_intel do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.6.0/zeno-0.6.0-linux-x86_64.tar.gz"
      sha256 "202bf60f0d953d71ec1fd8a31c20ce6aed3bb5da6b0c762e67945b63b92737dc"
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
