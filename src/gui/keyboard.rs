use crate::keymap::parse_single_key;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use super::EguiApp;
const KEYBOARD_FRAME_WIDTH: f32 = 980.0;
const KEYBOARD_FRAME_HEIGHT: f32 = 240.0;
const KEYBOARD_KEY_WIDTH: f32 = 36.0;
const KEYBOARD_KEY_HEIGHT: f32 = 30.0;
const KEYBOARD_KEY_GAP: f32 = 4.0;
const KEYBOARD_BLOCK_GAP: f32 = 12.0;
const KEYBOARD_MARGIN: f32 = 12.0;
const KEYBOARD_BG_SELECTED: Color32 = Color32::from_rgb(191, 221, 255);
const KEYBOARD_BG_NORMAL: Color32 = Color32::from_rgb(239, 243, 248);
const KEYBOARD_BG_DISABLED: Color32 = Color32::from_rgb(228, 228, 228);
const KEYBOARD_BORDER: Color32 = Color32::from_rgb(120, 132, 148);
const KEYBOARD_TEXT_DISABLED: Color32 = Color32::from_rgb(125, 125, 125);
const KEYBOARD_TEXT_NORMAL: Color32 = Color32::from_rgb(40, 40, 40);

#[derive(Clone, Copy)]
struct KeyboardLayoutKey {
    token: &'static str,
    label: &'static str,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Clone, Copy)]
struct KeyboardCell {
    token: &'static str,
    label: &'static str,
    width_units: f32,
}

impl EguiApp {
    pub(super) fn render_keyboard_panel(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("按键设置");
                ui.add_space(12.0);
                ui.label(
                    RichText::new(
                        "点击键帽切换是否加入连发。选中的键会保持高亮底色；右上角灰色键仅展示布局，不参与配置。",
                    )
                    .color(Color32::from_rgb(80, 88, 98))
                    .size(13.0),
                );
            });
            ui.add_space(8.0);

            let available = ui.available_size();
            let desired_height = available.y.max(220.0);
            let (response, painter) = ui.allocate_painter(
                Vec2::new(ui.available_width(), desired_height),
                Sense::click(),
            );
            let outer_rect = response.rect.shrink2(Vec2::splat(10.0));
            let scale = (outer_rect.width() / KEYBOARD_FRAME_WIDTH)
                .min(outer_rect.height() / KEYBOARD_FRAME_HEIGHT)
                .max(0.1);
            let scaled_size = Vec2::new(KEYBOARD_FRAME_WIDTH * scale, KEYBOARD_FRAME_HEIGHT * scale);
            let origin = Pos2::new(
                outer_rect.center().x - scaled_size.x / 2.0,
                outer_rect.center().y - scaled_size.y / 2.0,
            );

            let editable = self.state.is_editing_enabled();
            let layout_keys = keyboard_layout_keys();
            let mut hit_token = None;

            if editable && response.clicked() {
                if let Some(pointer_pos) = response.interact_pointer_pos() {
                    for key in &layout_keys {
                        let key_rect = scaled_key_rect(origin, scale, key);
                        if key_rect.contains(pointer_pos) && parse_single_key(key.token).is_ok() {
                            hit_token = Some(key.token.to_string());
                            break;
                        }
                    }
                }
            }

            for key in &layout_keys {
                let supported = parse_single_key(key.token).is_ok();
                let selected = supported && self.state.enabled_keys.contains(key.token);
                let key_rect = scaled_key_rect(origin, scale, key);
                let fill = if selected {
                    KEYBOARD_BG_SELECTED
                } else if supported {
                    KEYBOARD_BG_NORMAL
                } else {
                    KEYBOARD_BG_DISABLED
                };
                let text_color = if supported {
                    KEYBOARD_TEXT_NORMAL
                } else {
                    KEYBOARD_TEXT_DISABLED
                };
                let rounding = egui::CornerRadius::same((6.0 * scale).clamp(3.0, 8.0) as u8);

                painter.rect_filled(key_rect, rounding, fill);
                painter.rect_stroke(
                    key_rect,
                    rounding,
                    Stroke::new(if selected { 1.4 } else { 1.0 }, KEYBOARD_BORDER),
                    egui::StrokeKind::Outside,
                );
                painter.text(
                    key_rect.center(),
                    Align2::CENTER_CENTER,
                    key.label,
                    FontId::proportional((12.0 * scale).clamp(8.0, 18.0)),
                    text_color,
                );
            }

            if let Some(token) = hit_token {
                self.state.toggle_enabled_key(&token);
            }
        });
    }
}

fn scaled_key_rect(origin: Pos2, scale: f32, key: &KeyboardLayoutKey) -> Rect {
    Rect::from_min_size(
        Pos2::new(origin.x + key.x * scale, origin.y + key.y * scale),
        Vec2::new(key.width * scale, key.height * scale),
    )
}

fn keyboard_layout_keys() -> Vec<KeyboardLayoutKey> {
    let mut keys = Vec::new();
    let main_x = KEYBOARD_MARGIN;
    let nav_x = main_x + keyboard_units_to_px(16.0) + KEYBOARD_BLOCK_GAP;
    let num_x = nav_x + keyboard_units_to_px(3.0) + KEYBOARD_BLOCK_GAP;

    let row0 = keyboard_row_y(0.0);
    push_keyboard_row(
        &mut keys,
        main_x,
        row0,
        &[KeyboardCell {
            token: "ESC",
            label: "Esc",
            width_units: 1.0,
        }],
    );
    push_keyboard_row(
        &mut keys,
        main_x + keyboard_units_to_px(2.0),
        row0,
        &[
            KeyboardCell {
                token: "F1",
                label: "F1",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F2",
                label: "F2",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F3",
                label: "F3",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F4",
                label: "F4",
                width_units: 1.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        main_x + keyboard_units_to_px(6.0) + KEYBOARD_BLOCK_GAP,
        row0,
        &[
            KeyboardCell {
                token: "F5",
                label: "F5",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F6",
                label: "F6",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F7",
                label: "F7",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F8",
                label: "F8",
                width_units: 1.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        main_x + keyboard_units_to_px(10.0) + KEYBOARD_BLOCK_GAP * 2.0,
        row0,
        &[
            KeyboardCell {
                token: "F9",
                label: "F9",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F10",
                label: "F10",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F11",
                label: "F11",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F12",
                label: "F12",
                width_units: 1.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x,
        row0,
        &[
            KeyboardCell {
                token: "PRINTSCREEN",
                label: "PrtSc",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "SCROLLLOCK",
                label: "ScrLk",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "PAUSE",
                label: "Pause",
                width_units: 1.0,
            },
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(1.0),
        &[
            KeyboardCell {
                token: "BACKQUOTE",
                label: "~",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "1",
                label: "1",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "2",
                label: "2",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "3",
                label: "3",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "4",
                label: "4",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "5",
                label: "5",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "6",
                label: "6",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "7",
                label: "7",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "8",
                label: "8",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "9",
                label: "9",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "0",
                label: "0",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "MINUS",
                label: "-",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "EQUAL",
                label: "=",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "BACKSPACE",
                label: "Bksp",
                width_units: 2.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x,
        keyboard_row_y(1.0),
        &[
            KeyboardCell {
                token: "INSERT",
                label: "Ins",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "HOME",
                label: "Home",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "PAGEUP",
                label: "PgUp",
                width_units: 1.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(1.0),
        &[
            KeyboardCell {
                token: "NUMLOCK",
                label: "Num",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUMDIV",
                label: "/",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUMMUL",
                label: "*",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUMMINUS",
                label: "-",
                width_units: 1.0,
            },
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(2.0),
        &[
            KeyboardCell {
                token: "TAB",
                label: "Tab",
                width_units: 2.0,
            },
            KeyboardCell {
                token: "Q",
                label: "Q",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "W",
                label: "W",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "E",
                label: "E",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "R",
                label: "R",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "T",
                label: "T",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "Y",
                label: "Y",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "U",
                label: "U",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "I",
                label: "I",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "O",
                label: "O",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "P",
                label: "P",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "LBRACKET",
                label: "[",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "RBRACKET",
                label: "]",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "BACKSLASH",
                label: "\\",
                width_units: 2.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x,
        keyboard_row_y(2.0),
        &[
            KeyboardCell {
                token: "DELETE",
                label: "Del",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "END",
                label: "End",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "PAGEDOWN",
                label: "PgDn",
                width_units: 1.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(2.0),
        &[
            KeyboardCell {
                token: "NUM7",
                label: "7",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUM8",
                label: "8",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUM9",
                label: "9",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUMPLUS",
                label: "+",
                width_units: 1.0,
            },
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(3.0),
        &[
            KeyboardCell {
                token: "CAPSLOCK",
                label: "Caps",
                width_units: 2.0,
            },
            KeyboardCell {
                token: "A",
                label: "A",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "S",
                label: "S",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "D",
                label: "D",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "F",
                label: "F",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "G",
                label: "G",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "H",
                label: "H",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "J",
                label: "J",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "K",
                label: "K",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "L",
                label: "L",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "SEMICOLON",
                label: ";",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "APOSTROPHE",
                label: "'",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "ENTER",
                label: "Enter",
                width_units: 3.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(3.0),
        &[
            KeyboardCell {
                token: "NUM4",
                label: "4",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUM5",
                label: "5",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUM6",
                label: "6",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUMENTER",
                label: "Ent",
                width_units: 1.0,
            },
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(4.0),
        &[
            KeyboardCell {
                token: "LSHIFT",
                label: "LShift",
                width_units: 3.0,
            },
            KeyboardCell {
                token: "Z",
                label: "Z",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "X",
                label: "X",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "C",
                label: "C",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "V",
                label: "V",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "B",
                label: "B",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "N",
                label: "N",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "M",
                label: "M",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "COMMA",
                label: ",",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "PERIOD",
                label: ".",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "SLASH",
                label: "/",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "RSHIFT",
                label: "RShift",
                width_units: 3.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x + keyboard_units_to_px(1.0),
        keyboard_row_y(4.0),
        &[KeyboardCell {
            token: "UP",
            label: "Up",
            width_units: 1.0,
        }],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(4.0),
        &[
            KeyboardCell {
                token: "NUM1",
                label: "1",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUM2",
                label: "2",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "NUM3",
                label: "3",
                width_units: 1.0,
            },
        ],
    );

    push_keyboard_row(
        &mut keys,
        main_x,
        keyboard_row_y(5.0),
        &[
            KeyboardCell {
                token: "LCTRL",
                label: "LCtrl",
                width_units: 2.0,
            },
            KeyboardCell {
                token: "LWIN",
                label: "Win",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "LALT",
                label: "LAlt",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "SPACE",
                label: "Space",
                width_units: 6.0,
            },
            KeyboardCell {
                token: "RALT",
                label: "RAlt",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "RWIN",
                label: "Win",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "MENU",
                label: "Menu",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "RCTRL",
                label: "RCtrl",
                width_units: 3.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        nav_x,
        keyboard_row_y(5.0),
        &[
            KeyboardCell {
                token: "LEFT",
                label: "Left",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "DOWN",
                label: "Down",
                width_units: 1.0,
            },
            KeyboardCell {
                token: "RIGHT",
                label: "Right",
                width_units: 1.0,
            },
        ],
    );
    push_keyboard_row(
        &mut keys,
        num_x,
        keyboard_row_y(5.0),
        &[
            KeyboardCell {
                token: "NUM0",
                label: "0",
                width_units: 2.0,
            },
            KeyboardCell {
                token: "NUMDOT",
                label: ".",
                width_units: 1.0,
            },
        ],
    );

    keys
}

fn push_keyboard_row(keys: &mut Vec<KeyboardLayoutKey>, base_x: f32, y: f32, row: &[KeyboardCell]) {
    let mut x = base_x;
    for cell in row {
        keys.push(KeyboardLayoutKey {
            token: cell.token,
            label: cell.label,
            x,
            y,
            width: keyboard_units_to_px(cell.width_units),
            height: KEYBOARD_KEY_HEIGHT,
        });
        x += keyboard_units_to_px(cell.width_units) + KEYBOARD_KEY_GAP;
    }
}

fn keyboard_row_y(row: f32) -> f32 {
    KEYBOARD_MARGIN + row * (KEYBOARD_KEY_HEIGHT + KEYBOARD_KEY_GAP)
}

fn keyboard_units_to_px(units: f32) -> f32 {
    units * KEYBOARD_KEY_WIDTH + (units - 1.0) * KEYBOARD_KEY_GAP
}
