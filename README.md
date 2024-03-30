# vk-firewood-rs

Vulkan Bootstrapping library for Rust, inspired by vk-bootstrap based on [`ash-bootstrap`]/[`erupt-bootstrap`].

The original ash-bootstrap library hasn't been updated for years, contains to link to the erupt-bootstrap's crates.io page, and seems to have some issues that need resolving.

This fork intends to fix a few issues with ash-bootstrap and make it compatiable with [vkguide.dev](https://vkguide.dev).

- ✅ Instance creation
- ✅ Physical Device selection
- ✅ Device creation
- ✅ Getting queues
- ✅ Swapchain handling was handled in erupt-bootstrap, but ash_window takes care of creation, we handle swapchain creation/recreation in this lib.

## Cargo Features

- `surface` (enabled by default): Enables the use of [`raw-window-handle`].

## Example

see the examples dir for up to date examples

## Licensing

[This project is licensed under the MIT License.](https://github.com/SimonBorghese/vk-firewood-rs/blob/main/LICENSE)

[`ash-bootstrap`], the original fork of erupt-bootstrap made for Ash.

[`vk-bootstrap`], the inspiration of this project, is licensed under the [MIT license].

[`erupt-bootsrap`] is the main initial work of this by Friz64.  He's the real hero, I just work here.

[zlib License]: https://gitlab.com/Friz64/erupt-bootstrap/-/blob/main/LICENSE
[MIT license]: https://gitlab.com/Friz64/erupt-bootstrap/-/blob/main/LICENSE-vk-bootstrap
[`ash-bootstrap`]: https://github.com/brandonpollack23/ash-bootstrap
[`erupt-bootstrap`]: https://gitlab.com/Friz64/erupt-bootstrap
[`vk-bootstrap`]: https://github.com/charles-lunarg/vk-bootstrap
[`raw-window-handle`]: https://crates.io/crates/raw-window-handle
