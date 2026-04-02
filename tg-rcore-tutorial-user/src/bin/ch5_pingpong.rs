#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{console_getchar_nonblocking, fb_flush, fb_info, mmap, munmap};

const WALL_T: isize = 3;
const PADDLE_W: isize = 8;
const PADDLE_MARGIN: isize = 24;
const PAGE_SIZE: usize = 4096;
const FB_MAP_BASE: usize = 0x3000_0000;

#[derive(Clone, Copy)]
struct GameState {
    left_y: isize,
    right_y: isize,
    ball_x: isize,
    ball_y: isize,
    left_score: u8,
    right_score: u8,
    running: bool,
}

impl GameState {
    fn new(fb_w: usize, fb_h: usize) -> Self {
        Self {
            left_y: fb_h as isize / 2,
            right_y: fb_h as isize / 2,
            ball_x: fb_w as isize / 2,
            ball_y: fb_h as isize / 2,
            left_score: 0,
            right_score: 0,
            running: true,
        }
    }
}

fn paddle_h_px(fb_h: usize) -> isize {
    ((fb_h as isize) / 4).clamp(48, 120)
}

fn ball_r_px(fb_w: usize, fb_h: usize) -> isize {
    ((fb_w.min(fb_h) as isize) / 100).clamp(4, 10)
}

fn ball_speed_x_px(fb_w: usize) -> isize {
    ((fb_w as isize) / 96).clamp(8, 16)
}

fn ball_speed_y_px(fb_h: usize) -> isize {
    ((fb_h as isize) / 220).clamp(3, 8)
}

fn paddle_step_px(fb_w: usize, fb_h: usize) -> isize {
    let base = ((fb_h as isize) / 64).clamp(8, 18);
    base.max(ball_speed_x_px(fb_w)).max(ball_speed_y_px(fb_h))
}

fn clamp_paddle(y: isize, fb_h: usize, paddle_h: isize) -> isize {
    let half = paddle_h / 2;
    let min_y = WALL_T + half;
    let max_y = (fb_h as isize - WALL_T - 1) - half;
    if y < min_y {
        min_y
    } else if y > max_y {
        max_y
    } else {
        y
    }
}

fn reset_ball(
    state: &mut GameState,
    vx: &mut isize,
    vy: &mut isize,
    toward_left: bool,
    fb_w: usize,
    fb_h: usize,
) {
    state.ball_x = fb_w as isize / 2;
    state.ball_y = fb_h as isize / 2;
    let sx = ball_speed_x_px(fb_w);
    let sy = ball_speed_y_px(fb_h);
    *vx = if toward_left { -sx } else { sx };
    *vy = if *vy >= 0 { sy } else { -sy };
}

fn draw_rect(fb: &mut [u32], fb_w: usize, fb_h: usize, x: isize, y: isize, w: usize, h: usize, c: u32) {
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = (x + w as isize).min(fb_w as isize).max(0) as usize;
    let y1 = (y + h as isize).min(fb_h as isize).max(0) as usize;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    for yy in y0..y1 {
        let row = yy * fb_w;
        for xx in x0..x1 {
            fb[row + xx] = c;
        }
    }
}

fn draw_digit_7seg(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    x: isize,
    y: isize,
    scale: usize,
    digit: u8,
    color: u32,
) {
    let t = scale.max(2);
    let w = t * 4;
    let h = t * 7;
    let seg = match digit {
        0 => [true, true, true, true, true, true, false],
        1 => [false, true, true, false, false, false, false],
        2 => [true, true, false, true, true, false, true],
        3 => [true, true, true, true, false, false, true],
        4 => [false, true, true, false, false, true, true],
        5 => [true, false, true, true, false, true, true],
        6 => [true, false, true, true, true, true, true],
        7 => [true, true, true, false, false, false, false],
        8 => [true, true, true, true, true, true, true],
        9 => [true, true, true, true, false, true, true],
        _ => [false, false, false, false, false, false, false],
    };

    if seg[0] {
        draw_rect(fb, fb_w, fb_h, x, y, w, t, color);
    }
    if seg[1] {
        draw_rect(fb, fb_w, fb_h, x + (w - t) as isize, y, t, h / 2, color);
    }
    if seg[2] {
        draw_rect(
            fb,
            fb_w,
            fb_h,
            x + (w - t) as isize,
            y + (h / 2) as isize,
            t,
            h / 2,
            color,
        );
    }
    if seg[3] {
        draw_rect(fb, fb_w, fb_h, x, y + (h - t) as isize, w, t, color);
    }
    if seg[4] {
        draw_rect(fb, fb_w, fb_h, x, y + (h / 2) as isize, t, h / 2, color);
    }
    if seg[5] {
        draw_rect(fb, fb_w, fb_h, x, y, t, h / 2, color);
    }
    if seg[6] {
        draw_rect(fb, fb_w, fb_h, x, y + (h / 2 - t / 2) as isize, w, t, color);
    }
}

fn draw_scoreboard(fb: &mut [u32], fb_w: usize, fb_h: usize, left: u8, right: u8) {
    let left_d = left % 10;
    let right_d = right % 10;
    let scale = (fb_h / 120).max(2);
    let digit_w = scale * 4;
    let gap = scale * 2;
    let group_w = digit_w * 2 + gap * 3;
    let start_x = (fb_w.saturating_sub(group_w)) as isize / 2;
    let y = (scale * 2) as isize;

    draw_rect(
        fb,
        fb_w,
        fb_h,
        start_x - gap as isize,
        y - gap as isize,
        group_w + gap * 2,
        scale * 11,
        0xff20_2028,
    );

    draw_digit_7seg(fb, fb_w, fb_h, start_x, y, scale, left_d, 0xffff_5050);
    draw_rect(
        fb,
        fb_w,
        fb_h,
        start_x + (digit_w + gap) as isize,
        y + (scale * 2) as isize,
        scale,
        scale,
        0xffd0_d0d0,
    );
    draw_rect(
        fb,
        fb_w,
        fb_h,
        start_x + (digit_w + gap) as isize,
        y + (scale * 5) as isize,
        scale,
        scale,
        0xffd0_d0d0,
    );
    draw_digit_7seg(
        fb,
        fb_w,
        fb_h,
        start_x + (digit_w + gap * 2) as isize,
        y,
        scale,
        right_d,
        0xff50_80ff,
    );
}

fn render_gpu(state: GameState, fb: &mut [u32], fb_w: usize, fb_h: usize) {
    fb.fill(0xff10_1820);

    let paddle_h = paddle_h_px(fb_h);
    let ball_r = ball_r_px(fb_w, fb_h);
    let left_x = PADDLE_MARGIN;
    let right_x = fb_w as isize - PADDLE_MARGIN - PADDLE_W;

    draw_rect(fb, fb_w, fb_h, 0, 0, fb_w, WALL_T as usize, 0xffe0_e0e0);
    draw_rect(
        fb,
        fb_w,
        fb_h,
        0,
        fb_h as isize - WALL_T,
        fb_w,
        WALL_T as usize,
        0xffe0_e0e0,
    );

    for y in (0..fb_h).step_by(12) {
        draw_rect(fb, fb_w, fb_h, (fb_w / 2 - 1) as isize, y as isize, 2, 6, 0xff40_7090);
    }

    let ly = state.left_y - (paddle_h / 2);
    let ry = state.right_y - (paddle_h / 2);
    draw_rect(
        fb,
        fb_w,
        fb_h,
        left_x,
        ly,
        PADDLE_W as usize,
        paddle_h as usize,
        0xffff_4040,
    );
    draw_rect(
        fb,
        fb_w,
        fb_h,
        right_x,
        ry,
        PADDLE_W as usize,
        paddle_h as usize,
        0xff40_80ff,
    );

    draw_rect(
        fb,
        fb_w,
        fb_h,
        state.ball_x - ball_r,
        state.ball_y - ball_r,
        (ball_r * 2 + 1) as usize,
        (ball_r * 2 + 1) as usize,
        0xffff_d050,
    );

    let left_bar = ((state.left_score as usize) * (fb_w / 24)).min(fb_w / 4);
    let right_bar = ((state.right_score as usize) * (fb_w / 24)).min(fb_w / 4);
    draw_rect(fb, fb_w, fb_h, 12, 8, left_bar, 4, 0xffb0_4040);
    draw_rect(fb, fb_w, fb_h, (fb_w - 12 - right_bar) as isize, 8, right_bar, 4, 0xff40_70b0);
    draw_scoreboard(fb, fb_w, fb_h, state.left_score, state.right_score);

    let ret = fb_flush(fb.as_ptr() as *const u8, fb.len() * core::mem::size_of::<u32>());
    if ret != 0 {
        println!("ch5_pingpong: fb_flush failed: {}", ret);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    let mut fb_w = 0u32;
    let mut fb_h = 0u32;
    if fb_info(&mut fb_w as *mut u32, &mut fb_h as *mut u32) != 0 || fb_w == 0 || fb_h == 0 {
        println!("ch5_pingpong: gpu framebuffer unavailable");
        return -1;
    }
    let fb_bytes = (fb_w as usize) * (fb_h as usize) * core::mem::size_of::<u32>();
    let fb_map_len = (fb_bytes + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    if mmap(FB_MAP_BASE, fb_map_len, 0b11) != 0 {
        println!("ch5_pingpong: mmap framebuffer staging buffer failed");
        return -1;
    }
    let frame = unsafe {
        core::slice::from_raw_parts_mut(FB_MAP_BASE as *mut u32, (fb_w as usize) * (fb_h as usize))
    };

    println!("ch5_pingpong gpu {}x{}", fb_w, fb_h);
    println!("controls: W/S left paddle, A/D right paddle, Q quit");
    let mut latest = GameState::new(fb_w as usize, fb_h as usize);
    let mut vx: isize = ball_speed_x_px(fb_w as usize);
    let mut vy: isize = ball_speed_y_px(fb_h as usize);
    let paddle_step = paddle_step_px(fb_w as usize, fb_h as usize);
    let paddle_h = paddle_h_px(fb_h as usize);
    let ball_r = ball_r_px(fb_w as usize, fb_h as usize);
    let left_x = PADDLE_MARGIN;
    let right_x = fb_w as isize - PADDLE_MARGIN - PADDLE_W;
    let left_right = left_x + PADDLE_W - 1;
    let right_right = right_x + PADDLE_W - 1;
    let top_limit = WALL_T;
    let bottom_limit = fb_h as isize - WALL_T - 1;

    loop {
        let ch = console_getchar_nonblocking();
        if ch >= 0 {
            match ch as u8 {
                b'w' | b'W' => {
                    latest.left_y = clamp_paddle(latest.left_y - paddle_step, fb_h as usize, paddle_h)
                }
                b's' | b'S' => {
                    latest.left_y = clamp_paddle(latest.left_y + paddle_step, fb_h as usize, paddle_h)
                }
                b'a' | b'A' => {
                    latest.right_y = clamp_paddle(latest.right_y - paddle_step, fb_h as usize, paddle_h)
                }
                b'd' | b'D' => {
                    latest.right_y = clamp_paddle(latest.right_y + paddle_step, fb_h as usize, paddle_h)
                }
                b'q' | b'Q' => break,
                _ => {}
            }
        }

        latest.ball_x += vx;
        latest.ball_y += vy;

        if latest.ball_y - ball_r <= top_limit {
            latest.ball_y = top_limit + ball_r;
            vy = -vy;
        }
        if latest.ball_y + ball_r >= bottom_limit {
            latest.ball_y = bottom_limit - ball_r;
            vy = -vy;
        }

        let ball_left = latest.ball_x - ball_r;
        let ball_right = latest.ball_x + ball_r;
        let ball_top = latest.ball_y - ball_r;
        let ball_bottom = latest.ball_y + ball_r;
        let left_top = latest.left_y - paddle_h / 2;
        let left_bottom = left_top + paddle_h - 1;
        let right_top = latest.right_y - paddle_h / 2;
        let right_bottom = right_top + paddle_h - 1;

        if vx < 0 && ball_left <= left_right && ball_right >= left_x {
            if ball_bottom >= left_top && ball_top <= left_bottom {
                latest.ball_x = left_right + ball_r + 1;
                vx = -vx;
            } else {
                latest.right_score = latest.right_score.saturating_add(1);
                reset_ball(
                    &mut latest,
                    &mut vx,
                    &mut vy,
                    false,
                    fb_w as usize,
                    fb_h as usize,
                );
            }
        }

        if vx > 0 && ball_right >= right_x && ball_left <= right_right {
            if ball_bottom >= right_top && ball_top <= right_bottom {
                latest.ball_x = right_x - ball_r - 1;
                vx = -vx;
            } else {
                latest.left_score = latest.left_score.saturating_add(1);
                reset_ball(
                    &mut latest,
                    &mut vx,
                    &mut vy,
                    true,
                    fb_w as usize,
                    fb_h as usize,
                );
            }
        }

        render_gpu(latest, frame, fb_w as usize, fb_h as usize);
        for _ in 0..120_000 {
            core::hint::spin_loop();
        }
    }

    let _ = munmap(FB_MAP_BASE, fb_map_len);
    0
}
