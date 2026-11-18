# Zeno, for Homebrew. This file is written by tools/package.sh when a
# version is released: change that, not this.
#
#     brew tap zeno-src/zeno-lang https://github.com/zeno-src/zeno-lang
#     brew install zeno-src/zeno-lang/zeno
class Zeno < Formula
  desc "Small, readable, statically typed language for firmware"
  homepage "https://zeno-lang.org"
  version "0.5.2"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.5.2/zeno-0.5.2-macos-arm64.tar.gz"
      sha256 "13aaeae534924177b8ce2e4c767ca4fb1c950f24f45f3107df611474dd4e53f3"
    end
    on_intel do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.5.2/zeno-0.5.2-macos-x86_64.tar.gz"
      sha256 "be0a13a3d401fec94c840b182eea02aefce1f0b31273a7cb09a21d2a4175d25b"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.5.2/zeno-0.5.2-linux-arm64.tar.gz"
      sha256 "98cf21cef15bcb306124cd576a0714fdf1151063c6c72cba8f1cff534872d3b7"
    end
    on_intel do
      url "https://github.com/zeno-src/zeno-lang/releases/download/v0.5.2/zeno-0.5.2-linux-x86_64.tar.gz"
      sha256 "e1ca3b43061d091d6a5bc698046fec552417cb70252714d6b93afe74b85358d0"
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
