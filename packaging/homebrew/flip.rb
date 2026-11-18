# typed: strict
# frozen_string_literal: true

cask "flip" do
  version "0.1.0"
  sha256 "e4de6fc1c0f140ff7ecdec2c360cee09230dcfd4db0d660f66083ec82f77b029"

  url "https://github.com/thetinyfoundry/FLIP/releases/download/v#{version}/FLIP-#{version}-macOS-arm64.dmg"
  name "FLIP"
  desc "100% local Liquid Glass radial file converter and Create New File utility"
  homepage "https://github.com/thetinyfoundry/FLIP"

  depends_on arch: :arm64
  depends_on macos: ">= :sequoia"

  app "FLIP.app"
  binary "#{appdir}/FLIP.app/Contents/MacOS/flip"

  zap trash: [
    "~/Library/Application Support/FLIP",
    "~/Library/Caches/com.thetinyfoundry.flip.mac",
    "~/Library/LaunchAgents/com.thetinyfoundry.flip.mac.plist",
    "~/Library/Preferences/com.thetinyfoundry.flip.mac.plist",
    "~/Library/Services/Convert with FLIP.workflow",
    "~/Library/Services/New File with FLIP.workflow",
  ]
end
