# Zeno, for Homebrew. This file is written by tools/package.sh when a
# version is released: change that, not this.
#
#     brew tap partham20/zeno-lang https://github.com/partham20/zeno-lang
#     brew install partham20/zeno-lang/zeno
class Zeno < Formula
  desc "Small, readable, statically typed language for firmware"
  homepage "https://zeno-lang.org"
  version "0.2.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.2.0/zeno-0.2.0-macos-arm64.tar.gz"
      sha256 "e5694d6fea274f92f9b346c30d6dac28278bb609d6a405ba77da9fac994ae669"
    end
    on_intel do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.2.0/zeno-0.2.0-macos-x86_64.tar.gz"
      sha256 "3d11fcfb451b8f836b830063a360c9dac349420d94a83872042b9e8f56a32c04"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.2.0/zeno-0.2.0-linux-arm64.tar.gz"
      sha256 "d8f202e3735d57788a3c582e194c6d2de1b064570ef1c0c543bc5c397add22c1"
    end
    on_intel do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.2.0/zeno-0.2.0-linux-x86_64.tar.gz"
      sha256 "42b8e96c177ea27873ded9c04120e963219bb405bbff5dbe644fb167e5e8e095"
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
