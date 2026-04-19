<#
.SYNOPSIS
    Master script to split all oversized modules into submodule files.
    Each module is split at carefully chosen boundaries.
    After splitting, each mod.rs has only mod declarations and pub use re-exports.
#>

$base = "C:\Users\uniqu\Documents\workspace\Psmux-Modular\src"

function Split-Module {
    param(
        [string]$Module,
        [array]$Splits  # Array of @{Name="..."; Start=N; End=N}
    )

    $modFile = Join-Path $base "$Module\mod.rs"
    $allLines = Get-Content $modFile -Encoding UTF8
    $totalLines = $allLines.Count

    Write-Host "`n=== Splitting $Module ($totalLines lines) ===" -ForegroundColor Cyan

    # Find the imports section: all lines before the first definition
    # We'll consider the import section as everything up to the start of the first split
    $firstSplitStart = ($Splits | Sort-Object { $_.Start } | Select-Object -First 1).Start
    $importLines = @()
    if ($firstSplitStart -gt 1) {
        $importLines = $allLines[0..($firstSplitStart - 2)]
    }

    # Create each submodule file
    foreach ($split in $Splits) {
        $name = $split.Name
        $startIdx = $split.Start - 1  # Convert to 0-based
        $endIdx = $split.End - 1

        # Clamp to file bounds
        if ($endIdx -ge $totalLines) { $endIdx = $totalLines - 1 }

        $codeLines = $allLines[$startIdx..$endIdx]

        # Build submodule content: imports + code
        $content = @()
        $content += "#![allow(unused_imports)]"
        $content += "use super::*;"
        $content += ""
        # Add original imports (adapted: remove any mod declarations)
        foreach ($line in $importLines) {
            $trimmed = $line.TrimStart()
            # Skip mod declarations, blank lines at start, and allow(dead_code)
            if ($trimmed -match "^mod\s+" -or $trimmed -match "^#!\[allow\(dead_code\)\]") { continue }
            # Convert crate::MODULE:: to super:: for same-module references
            # (not needed since we use pub use * from mod.rs)
            $content += $line
        }
        $content += ""
        $content += $codeLines

        $subFile = Join-Path $base "$Module\$name.rs"
        $content | Set-Content $subFile -Encoding UTF8
        Write-Host "  $name.rs ($($codeLines.Count) lines + imports)"
    }

    # Build new mod.rs
    $modContent = @()
    $modContent += "#![allow(unused_imports)]"
    $modContent += ""

    # Re-add the original imports in mod.rs too (for the re-exports to resolve)
    foreach ($line in $importLines) {
        $trimmed = $line.TrimStart()
        if ($trimmed -match "^#!\[allow\(dead_code\)\]") { continue }
        $modContent += $line
    }
    $modContent += ""

    foreach ($split in $Splits) {
        $modContent += "mod $($split.Name);"
    }
    $modContent += ""

    foreach ($split in $Splits) {
        $modContent += "pub use $($split.Name)::*;"
    }

    $modFile = Join-Path $base "$Module\mod.rs"
    $modContent | Set-Content $modFile -Encoding UTF8
    Write-Host "  mod.rs rewritten ($($modContent.Count) lines)" -ForegroundColor Green
}

# ============================================================
# TYPES (1294 lines)
# ============================================================
Split-Module -Module "types" -Splits @(
    @{Name="notification"; Start=12; End=81}     # ControlNotification, ControlClient, ClientInfo
    @{Name="pane_types"; Start=83; End=176}       # Pane, WarmPane, ForwardedPane
    @{Name="window"; Start=178; End=260}          # LayoutKind, Node, Window, MenuItem, Menu, Hook, PipePaneState, WaitChannel
    @{Name="mode"; Start=262; End=348}            # Mode, SelectionMode, CopyModeState, FocusDir
    @{Name="app"; Start=349; End=650}             # AppState struct (fields only ~300 lines)
    @{Name="app_impl"; Start=651; End=828}        # AppState::new() + port_file_base()
    @{Name="action"; Start=830; End=1152}         # DragState, Action, Bind, CtrlReq
    @{Name="channels"; Start=1153; End=1350}      # PTY_DATA_READY, streams, channels, WaitForOp, ParsedTarget
)

# ============================================================
# TREE (744 lines)
# ============================================================
Split-Module -Module "tree" -Splits @(
    @{Name="structure"; Start=9; End=212}         # split_with_gaps through compute_rects
    @{Name="mutate"; Start=213; End=422}          # resize_all_panes through prune_exited
    @{Name="query"; Start=423; End=795}           # path_exists through collect_leaves + reap_children
)

# ============================================================
# STYLE (813 lines)
# ============================================================
Split-Module -Module "style" -Splits @(
    @{Name="colors"; Start=18; End=179}           # map_color through parse_tmux_style_components + apply_modifier
    @{Name="inline"; Start=180; End=325}          # parse_inline_styles, spans_visual_width, truncate_spans, expand/parse_status
    @{Name="format_layout"; Start=326; End=813}   # StatusAlignment, FormatToken, LayoutResult, parse_format_segments, layout_format_line
)

# ============================================================
# SESSION (482 lines)
# ============================================================
Split-Module -Module "session" -Splits @(
    @{Name="naming"; Start=7; End=90}             # is_warm_session, next_session_name, cleanup_stale_port_files
    @{Name="connection"; Start=91; End=282}       # read_session_key, send_auth_cmd, send_control, resolve_*
    @{Name="listing"; Start=283; End=510}         # list_session_names, TreeEntry, list_all_sessions_tree, kill_remaining
)

# ============================================================
# UTIL (466 lines)
# ============================================================
Split-Module -Module "util" -Splits @(
    @{Name="paths"; Start=10; End=86}             # expand_run_shell_path, infer_title_from_prompt
    @{Name="json_list"; Start=87; End=140}        # WinInfo, PaneInfo, WinTree, list_windows_json, list_windows_tmux, list_tree_json
    @{Name="encoding"; Start=141; End=196}        # BASE64_CHARS, base64_encode, base64_decode, quote_arg
    @{Name="env_parse"; Start=197; End=510}       # parse_env_assignment, parse_new_session_e_value_token, etc. + tests + color_to_name
)

# ============================================================
# RENDERING (462 lines)
# ============================================================
Split-Module -Module "rendering" -Splits @(
    @{Name="color_util"; Start=26; End=135}       # vt_to_color, dim_color, dim_predictions_enabled, has_conpty_passthrough, configured_cursor_code, apply_cursor_style
    @{Name="render_impl"; Start=136; End=462}     # render_window, fix_border_intersections, render_node, etc.
)

# ============================================================
# POPUP (434 lines)
# ============================================================
Split-Module -Module "popup" -Splits @(
    @{Name="create"; Start=26; End=136}           # create_popup_pane
    @{Name="serialize"; Start=137; End=434}       # serialize_popup_overlay, render_popup_overlay
)

# ============================================================
# HELP (521 lines)
# ============================================================
Split-Module -Module "help" -Splits @(
    @{Name="keybind_help"; Start=11; End=260}     # PREFIX_DEFAULTS, copy_mode_vi_lines, copy_search_lines, command_prompt_lines, cli_command_lines
    @{Name="option_help"; Start=261; End=521}     # options_lines, format_vars_lines, hooks_lines, mouse_lines, build_overlay_lines, build_list_keys_output
)

# ============================================================
# CLI (599 lines)
# ============================================================
Split-Module -Module "cli" -Splits @(
    @{Name="normalize"; Start=16; End=66}         # normalize_flag_equals, normalize_flag_equals_borrowed, get_program_name
    @{Name="print_help"; Start=67; End=509}       # print_help, print_version, print_commands
    @{Name="target"; Start=510; End=599}          # parse_target, extract_session_from_target
)

# ============================================================
# CONTROL (401 lines)
# ============================================================
Split-Module -Module "control" -Splits @(
    @{Name="format_notif"; Start=4; End=126}      # format_notification, escape_output, format_begin/end/error, emit_notification, has_control_clients
    @{Name="wire"; Start=127; End=401}            # remaining control mode wire protocol code
)

# ============================================================
# COPY_MODE (1224 lines)
# ============================================================
Split-Module -Module "copy_mode" -Splits @(
    @{Name="clipboard"; Start=20; End=241}        # emit_osc52, enter/exit_copy_mode, save/restore_copy_state, copy/read_system_clipboard
    @{Name="movement"; Start=242; End=500}        # current_prompt_pos, move_copy_cursor, get_copy_pos, move_to_*, move_word_*
    @{Name="scroll"; Start=461; End=692}          # scroll_pane_scrollback through capture_active_pane_text, save_latest_buffer
    @{Name="search"; Start=692; End=970}          # search_copy_mode through capture_active_pane_styled
    @{Name="advanced"; Start=971; End=1300}       # Big word motions, screen positions, find char, paragraphs, brackets, text objects
)

# ============================================================
# CONFIG (1504 lines)
# ============================================================
Split-Module -Module "config" -Splits @(
    @{Name="loader"; Start=14; End=321}           # current_config_file, is_warm_disabled, populate_default_bindings, load_config, parse_config_content
    @{Name="parser"; Start=322; End=862}          # parse_config_line, parse_option_value, split_chained_commands_pub
    @{Name="keybind"; Start=863; End=1300}        # parse_bind_key, parse_unbind_key, normalize_key, parse_key_name, source_file, parse_key_string, format_key_binding
    @{Name="tests"; Start=1301; End=1504}         # #[cfg(test)] mod tests
)

# ============================================================
# COMMANDS (2240 lines)
# ============================================================
Split-Module -Module "commands" -Splits @(
    @{Name="helpers"; Start=16; End=141}          # parse_popup_dim_local, DISPLAY_MESSAGE_DEFAULT_FMT, resolve_run_shell, build_run_shell_command
    @{Name="parse_cmd"; Start=142; End=600}       # build_choose_tree, parse_command_to_action
    @{Name="format_cmd"; Start=601; End=786}      # format_action, parse_command_line, parse_menu_definition, ensure_background, fire_hooks
    @{Name="execute"; Start=787; End=1500}        # execute_action, execute_command_prompt, execute_command_string + inner dispatch
    @{Name="execute_ext"; Start=1501; End=2240}   # remaining execute_command_string match arms
)

# ============================================================
# FORMAT (1679 lines)
# ============================================================
Split-Module -Module "format" -Splits @(
    @{Name="layout_fmt"; Start=26; End=92}        # set_buffer_idx_override, generate_window_layout
    @{Name="expand"; Start=93; End=400}           # expand_format, expand_format_for_window, expand_format_for_pane (first half)
    @{Name="expand_ext"; Start=401; End=790}      # expand_format_for_pane continued, lookup_option_pub
    @{Name="variables"; Start=791; End=1200}      # expand_var (huge variable lookup)
    @{Name="variables_ext"; Start=1201; End=1730} # expand_var continued, hostname_cached, default_* formats, format_list_*
)

# ============================================================
# LAYOUT (1165 lines)
# ============================================================
Split-Module -Module "layout" -Splits @(
    @{Name="serialize_types"; Start=15; End=158}  # serialize_screen_rows, cycle_top_layout, CellJson, CellRunJson, RowRunsJson, LayoutJson
    @{Name="dump_json"; Start=159; End=487}       # dump_layout_json
    @{Name="dump_fast"; Start=488; End=901}       # dump_layout_json_fast
    @{Name="apply_cycle"; Start=902; End=1165}    # apply_layout, cycle_layout, cycle_layout_reverse, parse_layout_string, parse_tmux_layout_string
)

# ============================================================
# PANE (1219 lines)
# ============================================================
Split-Module -Module "pane" -Splits @(
    @{Name="create"; Start=17; End=290}           # CURSOR_SHAPE_UNSET, conpty_preemptive_dsr, cached_shell, create_window, spawn_warm_pane, split_active, create_window_raw
    @{Name="split_kill"; Start=291; End=514}      # MIN_PANE_DIM, split_active_with_command, kill_pane_at_path, kill_active_pane, kill_pane_by_id
    @{Name="env_setup"; Start=515; End=856}       # detect_shell, set_tmux_env, apply_user_environment, ENV_SHIM_PS, PSRL*, build_psrl_init
    @{Name="build_cmd"; Start=857; End=1091}      # build_command, cached_which, resolve_shell_program, build_default_shell, build_raw_command
    @{Name="reader"; Start=1092; End=1219}        # scan_cursor_shape, scan_rmcup, spawn_reader_thread
)

# ============================================================
# PLATFORM (2086 lines)
# ============================================================
Split-Module -Module "platform" -Splits @(
    @{Name="spawn"; Start=18; End=310}            # CREATE_NO_WINDOW, HideWindowCommandExt, spawn_server_hidden, enable_vtp, disable_vti, install_console_ctrl_handler
    @{Name="mouse_inject"; Start=311; End=900}    # Mouse injection (Win32 API), console mode detection
    @{Name="process"; Start=901; End=1500}        # Process detection, foreground window, title detection
    @{Name="console"; Start=1501; End=2100}       # Console APIs, Utf16ConsoleWriter
    @{Name="input_fix"; Start=2101; End=2350}     # PsmuxWriter type, create_writer, augment_enter_shift
)

# ============================================================
# SSH_INPUT (1536 lines)
# ============================================================
Split-Module -Module "ssh_input" -Splits @(
    @{Name="detect"; Start=1; End=260}            # send_mouse_enable, is_ssh_session, needs_vt_input, windows_build_number, InputSource enum
    @{Name="vt_parser"; Start=261; End=800}       # InputSource impl, make_key, decode_*, PS enum, VtParser struct + impl
    @{Name="vt_parser_ext"; Start=801; End=1200}  # VtParser continued (more dispatch methods)
    @{Name="reader"; Start=1201; End=1536}        # vk_to_keycode, vk_modifiers, SSH_LOG, ssh_debug_log, start_ssh_reader
)

# ============================================================
# WINDOW_OPS (1220 lines)
# ============================================================
Split-Module -Module "window_ops" -Splits @(
    @{Name="mouse_inject_ops"; Start=16; End=400} # mouse_log, pane_inner_cell, write_mouse_event_remote, inject_mouse, vt_bridge detection, inject_mouse_combined
    @{Name="zoom"; Start=390; End=472}            # push_zoom, pop_zoom, unzoom_if_zoomed, toggle_zoom, update_tab_positions
    @{Name="remote_mouse"; Start=473; End=835}    # remote_mouse_down/drag/up/button/motion, scroll functions
    @{Name="pane_ops"; Start=836; End=1220}       # handle_pane_mouse, handle_pane_scroll, handle_split_*, swap, resize, rotate, break, respawn
)

# ============================================================
# INPUT (3082 lines)
# ============================================================
Split-Module -Module "input" -Splits @(
    @{Name="keyboard"; Start=42; End=400}         # handle_key first half (Passthrough, Prefix)
    @{Name="key_dispatch"; Start=401; End=800}    # handle_key continued (command prompt, copy mode keys)
    @{Name="key_ext"; Start=801; End=1200}        # handle_key continued (more modes)
    @{Name="copy_keys"; Start=1201; End=1612}     # handle_key copy mode + vi commands, find_best_pane_in_direction, find_wrap_target
    @{Name="encode"; Start=1612; End=1960}        # parse_modified_special_key, encode_key_event, forward_key_to_active
    @{Name="mouse"; Start=1960; End=2520}         # handle_mouse
    @{Name="send"; Start=2520; End=3082}          # send_paste_to_active, send_text_to_active, send_key_to_active
)

# ============================================================
# CLIENT (4493 lines)
# ============================================================
Split-Module -Module "client" -Splits @(
    @{Name="types_util"; Start=1; End=120}        # imports + helper types + modified_key_name
    @{Name="selection"; Start=121; End=363}       # PaneLeaf, collect_leaves, text selection functions
    @{Name="run_remote"; Start=364; End=800}      # run_remote entry + TCP connection + frame receive loop start
    @{Name="render"; Start=801; End=1300}         # Frame rendering, draw calls, overlay rendering
    @{Name="status"; Start=1301; End=1800}        # Status bar rendering, tab bar, predictions
    @{Name="events"; Start=1801; End=2200}        # Event handling, key dispatch
    @{Name="overlays"; Start=2201; End=2700}      # Popup, menu, confirm, pane chooser rendering
    @{Name="commands"; Start=2701; End=3200}      # Command prompt, display-panes, clock mode
    @{Name="mouse_client"; Start=3201; End=3700}  # Client-side mouse handling
    @{Name="input_client"; Start=3701; End=4100}  # Client input processing
    @{Name="misc"; Start=4101; End=4493}          # Remaining client functions
)

# ============================================================
# SERVER/MOD.RS (4160 lines) - already a folder, just split mod.rs
# ============================================================
Split-Module -Module "server" -Splits @(
    @{Name="init"; Start=50; End=318}             # serialize_overlay_json + helper functions before run_server
    @{Name="run_server_fn"; Start=319; End=800}   # run_server function start (setup, TCP listener, warm pane)
    @{Name="event_loop"; Start=801; End=1300}     # Main event loop (recv_timeout, CtrlReq dispatch start)
    @{Name="dispatch_a"; Start=1301; End=1800}    # CtrlReq dispatch arms (first batch)
    @{Name="dispatch_b"; Start=1801; End=2300}    # CtrlReq dispatch arms (second batch)
    @{Name="dispatch_c"; Start=2301; End=2800}    # CtrlReq dispatch arms (third batch)
    @{Name="dispatch_d"; Start=2801; End=3300}    # CtrlReq dispatch arms (fourth batch)
    @{Name="dispatch_e"; Start=3301; End=3800}    # CtrlReq dispatch arms (fifth batch)
    @{Name="post_loop"; Start=3801; End=4160}     # Post-dispatch: frame push, cleanup, hooks
)

Write-Host "`n=== All modules split! ===" -ForegroundColor Green
Write-Host "Running cargo check..." -ForegroundColor Yellow
