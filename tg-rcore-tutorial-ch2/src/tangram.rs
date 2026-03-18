pub struct Point {
    pub x: i32,
    pub y: i32,
}
pub struct Polygon {
    pub color: [u8; 4],
    pub vertices: &'static [Point],
}

const RED: [u8; 4] = [0, 0, 255, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const DEEPBLUE: [u8; 4] = [255, 0, 0, 255];
const YELLOW: [u8; 4] = [0, 255, 255, 255];
const PINK: [u8; 4] = [255, 0, 255, 255];
const BLUE: [u8; 4] = [255, 200, 0, 255];
const ORANGE: [u8; 4] = [0, 144, 255, 255];

// "O"
pub const TANGRAM_O: &[Polygon] = &[
    // 左上 红色等腰直角三角形
    Polygon {
        color: RED,
        vertices: &[
            Point { x: 80, y: 80 },
            Point { x: 240, y: 80 },
            Point { x: 80, y: 240 },
        ],
    },
    // 左侧 黄色平行四边形
    Polygon {
        color: YELLOW,
        vertices: &[
            Point { x: 80, y: 240 },
            Point { x: 80, y: 560 },
            Point { x: 240, y: 400 },
            Point { x: 240, y: 80 },
        ],
    },
    // 左下 蓝色等腰直角三角形
    Polygon {
        color: BLUE,
        vertices: &[
            Point { x: 80, y: 560 },
            Point { x: 80, y: 880 },
            Point { x: 400, y: 880 },
        ],
    },
    // 右下 绿色菱形
    Polygon {
        color: GREEN,
        vertices: &[
            Point { x: 400, y: 560 },
            Point { x: 240, y: 720 },
            Point { x: 400, y: 880 },
            Point { x: 560, y: 720 },
        ],
    },
    // 右侧 深蓝色平行四边形
    Polygon {
        color: DEEPBLUE,
        vertices: &[
            Point { x: 400, y: 240 },
            Point { x: 400, y: 560 },
            Point { x: 560, y: 720 },
            Point { x: 560, y: 400 },
        ],
    },
    // 右上 粉色等腰直角三角形
    Polygon {
        color: PINK,
        vertices: &[
            Point { x: 240, y: 80 },
            Point { x: 560, y: 400 },
            Point { x: 560, y: 80 },
        ],
    },
];

// "S" 图案
pub const TANGRAM_S: &[Polygon] = &[
    // 左上 蓝色等腰直角三角形
    Polygon {
        color: BLUE,
        vertices: &[
            Point { x: 880, y: 80 },
            Point { x: 720, y: 240 },
            Point { x: 880, y: 400 },
        ],
    },
    // 上部 深蓝色等腰直角三角形
    Polygon {
        color: DEEPBLUE,
        vertices: &[
            Point { x: 880, y: 80 },
            Point { x: 1040, y: 80 },
            Point { x: 1040, y: 240 },
        ],
    },
    // 右上 粉色梯形
    Polygon {
        color: PINK,
        vertices: &[
            Point { x: 1040, y: 240 },
            Point { x: 1040, y: 40 },
            Point { x: 1200, y: 40 },
            Point { x: 1200, y: 160 },
        ],
    },
    // 中部 绿色正方形
    Polygon {
        color: GREEN,
        vertices: &[
            Point { x: 880, y: 400 },
            Point { x: 1040, y: 400 },
            Point { x: 1040, y: 560 },
            Point { x: 880, y: 560 },
        ],
    },
    // 右侧 粉色等腰直角三角形
    Polygon {
        color: PINK,
        vertices: &[
            Point { x: 1040, y: 400 },
            Point { x: 1200, y: 560 },
            Point { x: 1040, y: 720 },
        ],
    },
    // 下部 深蓝色等腰三角形
    Polygon {
        color: DEEPBLUE,
        vertices: &[
            Point { x: 1040, y: 720 },
            Point { x: 960, y: 880 },
            Point { x: 880, y: 720 },
        ],
    },
    // 左下 橙色平行四边形
    Polygon {
        color: ORANGE,
        vertices: &[
            Point { x: 880, y: 720 },
            Point { x: 960, y: 880 },
            Point { x: 800, y: 880 },
            Point { x: 720, y: 720 },
        ],
    },
];

// 扫描线算法填充多边形
pub fn fill_polygon(vertices: &[Point], color: [u8; 4], fb: &mut [u8], width: u32) {
    let min_y = vertices.iter().map(|v| v.y).min().unwrap_or(0).max(0);
    let max_y = vertices.iter().map(|v| v.y).max().unwrap_or(0);
    let min_x = vertices.iter().map(|v| v.x).min().unwrap_or(0).max(0);
    let max_x = vertices.iter().map(|v| v.x).max().unwrap_or(0);

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let mut inside = false;
            let mut j = vertices.len() - 1;
            for i in 0..vertices.len() {
                let pi = &vertices[i];
                let pj = &vertices[j];

                if (pi.y > y) != (pj.y > y) {
                    let intersect_x = pi.x + (pj.x - pi.x) * (y - pi.y) / (pj.y - pi.y);
                    if x < intersect_x {
                        inside = !inside;
                    }
                }
                j = i;
            }

            if inside {
                let idx = ((y as usize) * (width as usize) + (x as usize)) * 4;
                if idx + 3 < fb.len() {
                    fb[idx] = color[0];
                    fb[idx + 1] = color[1];
                    fb[idx + 2] = color[2];
                    fb[idx + 3] = color[3];
                }
            }
        }
    }
}
