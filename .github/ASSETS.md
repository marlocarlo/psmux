# Repository Assets

## Social Card Setup

To enable the social card on GitHub:

1. **Convert SVG to PNG** (if not already done):
   - Online: Upload `.github/social-card.svg` to https://cloudconvert.com/svg-to-png
   - Or install ImageMagick: `winget install ImageMagick.ImageMagick`
   - Then run: `magick .github/social-card.svg .github/social-card.png`

2. **Upload to GitHub**:
   - Go to: https://github.com/psmux/psmux/settings
   - Scroll to "Social preview"
   - Click "Edit" and upload `.github/social-card.png`
   - Dimensions: 1280x640px (optimal for social sharing)

## Repository Icon

The `icon.svg` can be used as:
- Project logo in documentation
- Favicon for project websites
- App icon if building a GUI wrapper

### Design Features

**Social Card (`1280x640px`):**
- Dark gradient background (#1a1a2e → #16213e)
- Terminal window with split pane visualization
- psmux branding with cyan accent (#00d9ff)
- Feature badges: tmux-compatible, Windows-native, Rust-powered, No WSL
- PS> prompts to emphasize PowerShell support

**Icon (`512x512px`):**
- Cyan gradient circular background
- Terminal window with 3-pane split layout
- Animated cursor (blinks when viewed as SVG)
- Compact design suitable for various sizes

Both designs emphasize:
- Terminal multiplexing (split panes)
- Windows/PowerShell focus (PS> prompts)
- Modern, professional aesthetic
- Brand color consistency (#00d9ff cyan)
