// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
//  https://nautechsystems.io
//
//  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
//  You may not use this file except in compliance with the License.
//  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
// -------------------------------------------------------------------------------------------------

//! 数学函数和插值工具。
//!
//! 此模块提供了量化交易中必不可少的数学运算，
//! 包括金融数据处理和分析中常用的线性以及二次插值函数。
//!
//! # Epsilon 值 (Epsilon Values)
//!
//! 此模块使用了两个 epsilon 阈值：
//!
//! - **`f64::EPSILON * 2.0` (~4.44e-16):** 用于在 `linear_weight` 和 `quad_polynomial` 中探测近乎为零的分母，
//!   以防止除法不稳定性。这是一个机器精度级别的阈值。
//!
//! - **`1e-8`:** 用于 `quadratic_interpolation` 中探测精确的样本点。
//!   这是一个适用于典型金融数据应用级别的阈值。

/// 用于近似浮点数相等比较的宏。
///
/// 此宏使用指定的 epsilon 容差比较两个浮点值，
/// 为可能因浮点精度问题而失败的精确相等检查提供了一种安全的替代方案。
///
/// # 示例
///
/// ```rust
/// use nautilus_core::approx_eq;
///
/// let a = 0.1 + 0.2;
/// let b = 0.3;
/// assert!(approx_eq!(f64, a, b, epsilon = 1e-10));
/// ```
#[macro_export]
macro_rules! approx_eq {
    ($type:ty, $left:expr, $right:expr, epsilon = $epsilon:expr) => {{
        let left_val: $type = $left;
        let right_val: $type = $right;
        (left_val - right_val).abs() < $epsilon
    }};
}

/// 计算值 `x` 在 `x1` 和 `x2` 之间的插值权重。
///
/// 当对对应于横坐标 `x1` 和 `x2` 的纵坐标进行插值时，
/// 返回的权重 `w` 满足 `y = (1 - w) * y1 + w * y2`。
///
/// # Panics
///
/// - 如果任何输入为 NaN 或无穷大。
/// - 如果 `x1` 和 `x2` 距离太近（在机器 epsilon 范围内），这将导致除以零或数值不稳定。
#[inline]
#[must_use]
pub fn linear_weight(x1: f64, x2: f64, x: f64) -> f64 {
    const EPSILON: f64 = f64::EPSILON * 2.0; // ~4.44e-16

    assert!(
        x1.is_finite() && x2.is_finite() && x.is_finite(),
        "所有输入必须是有限值：x1={x1}, x2={x2}, x={x}"
    );

    let diff = (x2 - x1).abs();
    assert!(
        diff >= EPSILON,
        "`x1` ({x1}) 和 `x2` ({x2}) 距离太近，无法进行稳定的插值（差异：{diff}，最小值：{EPSILON}）"
    );
    (x - x1) / (x2 - x1)
}

/// 使用权重因子执行线性插值。
///
/// 给定纵坐标 `y1` 和 `y2` 以及权重 `x1_diff`，使用公式 `y1 + x1_diff * (y2 - y1)` 计算插值。
#[inline]
#[must_use]
pub fn linear_weighting(y1: f64, y2: f64, x1_diff: f64) -> f64 {
    x1_diff.mul_add(y2 - y1, y1)
}

/// 在有序数组中查找插值位置。
///
/// 返回 `xs` 中小于 `x` 的最大元素的索引，且限制在有效范围 `[0, xs.len() - 1]` 内。
///
/// # 边界情况
///
/// - 如果数组为空，返回 0
/// - 如果是单元素数组，无论 `x > xs[0]` 与否，始终返回索引 0
/// - 如果值低于最小值，返回 0
/// - 如果值等于或高于最大值，返回 `xs.len() - 1`
#[inline]
#[must_use]
pub fn pos_search(x: f64, xs: &[f64]) -> usize {
    if xs.is_empty() {
        return 0;
    }

    let n_elem = xs.len();
    let pos = xs.partition_point(|&val| val < x);
    std::cmp::min(std::cmp::max(pos.saturating_sub(1), 0), n_elem - 1)
}

/// 求解由三点定义的二次拉格朗日多项式 (Lagrange polynomial)。
///
/// 给定三点 `(x0, y0)`、`(x1, y1)`、`(x2, y2)`，此函数返回 *P(x)*，
/// 其中 *P* 是通过这三个点的唯一次数 ≤ 2 的多项式。
///
/// # Panics
///
/// - 如果任何输入为 NaN 或无穷大。
/// - 如果任意两个横坐标距离太近（在机器 epsilon 范围内），这将导致除以零或数值不稳定。
#[inline]
#[must_use]
pub fn quad_polynomial(x: f64, x0: f64, x1: f64, x2: f64, y0: f64, y1: f64, y2: f64) -> f64 {
    const EPSILON: f64 = f64::EPSILON * 2.0; // ~4.44e-16

    assert!(
        x.is_finite()
            && x0.is_finite()
            && x1.is_finite()
            && x2.is_finite()
            && y0.is_finite()
            && y1.is_finite()
            && y2.is_finite(),
        "所有输入必须是有限值：x={x}, x0={x0}, x1={x1}, x2={x2}, y0={y0}, y1={y1}, y2={y2}"
    );

    // 防止导致除以零的相同 x 值
    let diff_01 = (x0 - x1).abs();
    let diff_02 = (x0 - x2).abs();
    let diff_12 = (x1 - x2).abs();

    assert!(
        diff_01 >= EPSILON && diff_02 >= EPSILON && diff_12 >= EPSILON,
        "横坐标距离太近，无法进行稳定的插值：x0={x0}, x1={x1}, x2={x2}（差异：{diff_01:.2e}, {diff_02:.2e}, {diff_12:.2e}, 最小值：{EPSILON}）"
    );

    y0 * (x - x1) * (x - x2) / ((x0 - x1) * (x0 - x2))
        + y1 * (x - x0) * (x - x2) / ((x1 - x0) * (x1 - x2))
        + y2 * (x - x0) * (x - x1) / ((x2 - x0) * (x2 - x1))
}

/// 给定横坐标向量 `xs` 和纵坐标向量 `ys`，对点 `x` 执行二次插值。
///
/// # Panics
///
/// 如果 `xs.len() < 3` 或 `xs.len() != ys.len()`，则触发 panic。
#[must_use]
pub fn quadratic_interpolation(x: f64, xs: &[f64], ys: &[f64]) -> f64 {
    let n_elem = xs.len();
    let epsilon = 1e-8;

    assert!(
        n_elem >= 3,
        "二次插值至少需要 3 个点"
    );
    assert_eq!(xs.len(), ys.len(), "xs 和 ys 长度必须相等");

    if x <= xs[0] {
        return ys[0];
    }

    if x >= xs[n_elem - 1] {
        return ys[n_elem - 1];
    }

    let pos = pos_search(x, xs);

    if (xs[pos] - x).abs() < epsilon {
        return ys[pos];
    }

    if pos == 0 {
        return quad_polynomial(x, xs[0], xs[1], xs[2], ys[0], ys[1], ys[2]);
    }

    if pos == n_elem - 2 {
        return quad_polynomial(
            x,
            xs[n_elem - 3],
            xs[n_elem - 2],
            xs[n_elem - 1],
            ys[n_elem - 3],
            ys[n_elem - 2],
            ys[n_elem - 1],
        );
    }

    let w = linear_weight(xs[pos], xs[pos + 1], x);

    linear_weighting(
        quad_polynomial(
            x,
            xs[pos - 1],
            xs[pos],
            xs[pos + 1],
            ys[pos - 1],
            ys[pos],
            ys[pos + 1],
        ),
        quad_polynomial(
            x,
            xs[pos],
            xs[pos + 1],
            xs[pos + 2],
            ys[pos],
            ys[pos + 1],
            ys[pos + 2],
        ),
        w,
    )
}

#[cfg(test)]
mod tests {
    use rstest::*;

    use super::*;

    #[rstest]
    #[case(0.0, 10.0, 5.0, 0.5)]
    #[case(1.0, 3.0, 2.0, 0.5)]
    #[case(0.0, 1.0, 0.25, 0.25)]
    #[case(0.0, 1.0, 0.75, 0.75)]
    fn test_linear_weight_valid_cases(
        #[case] x1: f64,
        #[case] x2: f64,
        #[case] x: f64,
        #[case] expected: f64,
    ) {
        let result = linear_weight(x1, x2, x);
        assert!(
            approx_eq!(f64, result, expected, epsilon = 1e-10),
            "预期为 {expected}，实际为 {result}"
        );
    }

    #[rstest]
    #[should_panic(expected = "too close for stable interpolation")]
    fn test_linear_weight_zero_divisor() {
        let _ = linear_weight(1.0, 1.0, 0.5);
    }

    #[rstest]
    #[should_panic(expected = "too close for stable interpolation")]
    fn test_linear_weight_near_equal_values() {
        // 在机器 epsilon 范围内的值应被拒绝
        let _ = linear_weight(1.0, 1.0 + f64::EPSILON, 0.5);
    }

    #[rstest]
    fn test_linear_weight_with_small_differences() {
        // 高分辨率数据（例如，以秒为单位的纳秒时间戳）应当正常工作
        let result = linear_weight(0.0, 1e-12, 5e-13);
        assert!(result.is_finite());
        assert!((result - 0.5).abs() < 1e-10); // 结果应约为 0.5
    }

    #[rstest]
    fn test_linear_weight_just_above_epsilon() {
        // 差异超过机器 epsilon 的值应当正常工作
        let result = linear_weight(1.0, 1.0 + 1e-9, 1.0 + 5e-10);
        // 不应触发 panic 且应返回一个合理值
        assert!(result.is_finite());
    }

    #[rstest]
    #[case(1.0, 3.0, 0.5, 2.0)]
    #[case(10.0, 20.0, 0.25, 12.5)]
    #[case(0.0, 10.0, 0.0, 0.0)]
    #[case(0.0, 10.0, 1.0, 10.0)]
    fn test_linear_weighting(
        #[case] y1: f64,
        #[case] y2: f64,
        #[case] weight: f64,
        #[case] expected: f64,
    ) {
        let result = linear_weighting(y1, y2, weight);
        assert!(
            approx_eq!(f64, result, expected, epsilon = 1e-10),
            "预期为 {expected}，实际为 {result}"
        );
    }

    #[rstest]
    #[case(5.0, &[1.0, 2.0, 3.0, 4.0, 6.0, 7.0], 3)]
    #[case(1.5, &[1.0, 2.0, 3.0, 4.0], 0)]
    #[case(0.5, &[1.0, 2.0, 3.0, 4.0], 0)]
    #[case(4.5, &[1.0, 2.0, 3.0, 4.0], 3)]
    #[case(2.0, &[1.0, 2.0, 3.0, 4.0], 0)]
    fn test_pos_search(#[case] x: f64, #[case] xs: &[f64], #[case] expected: usize) {
        let result = pos_search(x, xs);
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_pos_search_edge_cases() {
        // 单个元素的数组
        let result = pos_search(5.0, &[10.0]);
        assert_eq!(result, 0);

        // 刚好位于边界处的值
        let result = pos_search(3.0, &[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(result, 1); // 小于 3.0 的最大元素索引为索引 1（值 2.0）

        // 两个元素的数组
        let result = pos_search(1.5, &[1.0, 2.0]);
        assert_eq!(result, 0);
    }

    #[rstest]
    fn test_pos_search_empty_slice() {
        let empty: &[f64] = &[];
        assert_eq!(pos_search(42.0, empty), 0);
    }

    #[rstest]
    fn test_quad_polynomial_linear_case() {
        // 使用三个共线点进行测试 - 应表现得像线性插值
        let result = quad_polynomial(1.5, 1.0, 2.0, 3.0, 1.0, 2.0, 3.0);
        assert!(approx_eq!(f64, result, 1.5, epsilon = 1e-10));
    }

    #[rstest]
    fn test_quad_polynomial_parabola() {
        // 使用简单的抛物线 y = x^2 进行测试
        // 点：(0,0), (1,1), (2,4)
        let result = quad_polynomial(1.5, 0.0, 1.0, 2.0, 0.0, 1.0, 4.0);
        let expected = 1.5 * 1.5; // 应为 2.25
        assert!(approx_eq!(f64, result, expected, epsilon = 1e-10));
    }

    #[rstest]
    #[should_panic(expected = "too close for stable interpolation")]
    fn test_quad_polynomial_duplicate_x() {
        let _ = quad_polynomial(0.5, 1.0, 1.0, 2.0, 0.0, 1.0, 4.0);
    }

    #[rstest]
    #[should_panic(expected = "too close for stable interpolation")]
    fn test_quad_polynomial_near_equal_x_values() {
        // x0 和 x1 的差异仅为机器 epsilon
        let _ = quad_polynomial(0.5, 1.0, 1.0 + f64::EPSILON, 2.0, 0.0, 1.0, 4.0);
    }

    #[rstest]
    fn test_quad_polynomial_with_small_differences() {
        // 高分辨率数据应当正常工作（例如，间隔为 1e-12）
        let result = quad_polynomial(5e-13, 0.0, 1e-12, 2e-12, 0.0, 1.0, 4.0);
        assert!(result.is_finite());
    }

    #[rstest]
    fn test_quad_polynomial_just_above_epsilon() {
        // 差异超过机器 epsilon 的值应当正常工作
        let result = quad_polynomial(0.5, 0.0, 1.0 + 1e-9, 2.0, 0.0, 1.0, 4.0);
        // 不应触发 panic 且应返回一个合理值
        assert!(result.is_finite());
    }

    #[rstest]
    fn test_quadratic_interpolation_boundary_conditions() {
        let xs = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let ys = vec![1.0, 4.0, 9.0, 16.0, 25.0]; // y = x^2

        // 测试低于最小值的情况
        let result = quadratic_interpolation(0.5, &xs, &ys);
        assert_eq!(result, ys[0]);

        // 测试高于最大值的情况
        let result = quadratic_interpolation(6.0, &xs, &ys);
        assert_eq!(result, ys[4]);
    }

    #[rstest]
    fn test_quadratic_interpolation_exact_points() {
        let xs = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let ys = vec![1.0, 4.0, 9.0, 16.0, 25.0];

        // 测试精确点点
        for (i, &x) in xs.iter().enumerate() {
            let result = quadratic_interpolation(x, &xs, &ys);
            assert!(approx_eq!(f64, result, ys[i], epsilon = 1e-6));
        }
    }

    #[rstest]
    fn test_quadratic_interpolation_intermediate_values() {
        let xs = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let ys = vec![1.0, 4.0, 9.0, 16.0, 25.0]; // y = x^2

        // 测试点间插值
        let result = quadratic_interpolation(2.5, &xs, &ys);
        let expected = 2.5 * 2.5; // 应接近 6.25
        assert!((result - expected).abs() < 0.1); // 允许一定的插值误差
    }

    #[rstest]
    #[should_panic(expected = "Need at least 3 points")]
    fn test_quadratic_interpolation_insufficient_points() {
        let xs = vec![1.0, 2.0];
        let ys = vec![1.0, 4.0];
        let _ = quadratic_interpolation(1.5, &xs, &ys);
    }

    #[rstest]
    #[should_panic(expected = "xs and ys must have the same length")]
    fn test_quadratic_interpolation_mismatched_lengths() {
        let xs = vec![1.0, 2.0, 3.0];
        let ys = vec![1.0, 4.0];
        let _ = quadratic_interpolation(1.5, &xs, &ys);
    }

    #[rstest]
    #[case(f64::NAN, 0.0, 1.0)]
    #[case(0.0, f64::NAN, 1.0)]
    #[case(0.0, 1.0, f64::NAN)]
    #[case(f64::INFINITY, 0.0, 1.0)]
    #[case(0.0, f64::NEG_INFINITY, 1.0)]
    #[should_panic(expected = "All inputs must be finite")]
    fn test_linear_weight_non_finite_panics(#[case] x1: f64, #[case] x2: f64, #[case] x: f64) {
        let _ = linear_weight(x1, x2, x);
    }

    #[rstest]
    #[should_panic(expected = "All inputs must be finite")]
    fn test_quad_polynomial_nan_panics() {
        let _ = quad_polynomial(f64::NAN, 0.0, 1.0, 2.0, 0.0, 1.0, 4.0);
    }

    #[rstest]
    #[should_panic(expected = "All inputs must be finite")]
    fn test_quad_polynomial_infinity_panics() {
        let _ = quad_polynomial(0.5, f64::INFINITY, 1.0, 2.0, 0.0, 1.0, 4.0);
    }
}
