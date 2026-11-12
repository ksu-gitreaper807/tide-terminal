Bundled, pinned upstream shell integrations. Their original licenses are retained.

- ble.sh: https://github.com/akinomyoga/ble.sh at 152942758f567e51fd17bdc7b3783d27b3c476aa (0.4.0-devel4), generated with its Makefile. Includes contrib at d2109203480a7dfe1dead5f5f8f9f15a9146c90d. Licenses: blesh/licenses/.
- zsh-autosuggestions: https://github.com/zsh-users/zsh-autosuggestions at e52ee8ca55bcc56a17c828767a3f98f22a68d4eb (v0.7.1). License: zsh-autosuggestions/LICENSE.

Enhanced suggestions can be disabled in the application's Settings. These plugins are loaded in application-launched interactive sessions; the application does not edit shell startup files.

- egui-winit 0.31.1: https://github.com/emilk/egui/tree/0.31.1/crates/egui-winit. MIT/Apache licenses are in egui-winit/. A local three-line behavioral patch retains Key events alongside Copy/Cut/Paste events instead of returning early. This lets the terminal receive Ctrl+C/X/V even with an empty clipboard. Tide filters the paired clipboard events only for terminal input; normal egui text fields retain clipboard behavior.
