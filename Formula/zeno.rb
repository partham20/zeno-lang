# Zeno, for Homebrew. This file is written by tools/package.sh when a
# version is released: change that, not this.
#
#     brew tap partham20/zeno-lang https://github.com/partham20/zeno-lang
#     brew install partham20/zeno-lang/zeno
class Zeno < Formula
  desc "Small, readable, statically typed language for firmware"
  homepage "https://zeno-lang.org"
  version "0.4.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.4.0/zeno-0.4.0-macos-arm64.tar.gz"
      sha256 "0fca22342e19cbbaa75c58bef6b1c61dd0f9ac8c396fce0c6364c7bedc019857"
    end
    on_intel do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.4.0/zeno-0.4.0-macos-x86_64.tar.gz"
      sha256 "47a9a99f1c604befac1c965d11e1cc3af293058677854a203c477213603d1527"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.4.0/zeno-0.4.0-linux-arm64.tar.gz"
      sha256 "51e2b7ceb3c0de3ae92eaa5b551ef85a568aa990d48c13d600965ad0e97ea116"
    end
    on_intel do
      url "https://github.com/partham20/zeno-lang/releases/download/v0.4.0/zeno-0.4.0-linux-x86_64.tar.gz"
      sha256 "9a95ead20fce96668ced41a25c09bc9b0c2db4f0160d4c6453352e759c7ae145"
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
