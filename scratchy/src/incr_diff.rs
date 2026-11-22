//! QINC1a (R60 sections 6.3 and 10): `style_diff`, the classifier the incremental engine uses to
//! decide which stage a restyled element feeds next. Nothing calls it yet (the engine lands in
//! QINC1f and later), so the module allows dead code until then.
//!
//! The classification table is ONE exhaustive destructuring of `ComputedStyle` (no `..`): adding a
//! field to `ComputedStyle` is a compile error here until the new field is given a class.
#![allow(dead_code)]

use super::*;
use std::fmt::Debug;

/// What a style change forces downstream, ordered: a bigger class includes the work of every
/// smaller one (`Box` rebuilds the box, `Layout` re-measures it, `Offset` re-places it, `Paint`
/// only rewrites the stored style).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum StyleDiff {
    /// Identical (apart from `node` and `tree_order`, which identify the element).
    None,
    /// Only painted values changed: written into the kept box and `Laid` without layout.
    Paint,
    /// Only the element's own offset changed (relative/absolute insets, translate): re-place it.
    Offset,
    /// The box must be re-measured (sizes, spacing, fonts, stacking boundaries, ...).
    Layout,
    /// The box content must be rebuilt (display, position, float, list-style, BFC-ness, anything
    /// the box builder branches on).
    Box,
}

/// One changed field of a `ComputedStyle`, for readable reports.
#[derive(Clone, Debug)]
pub(crate) struct FieldChange {
    pub(crate) field: &'static str,
    pub(crate) class: StyleDiff,
    pub(crate) old: String,
    pub(crate) new: String,
}

/// The callback of `style_walk`: field name, class, old value, new value.
type FieldFn<'a> = dyn FnMut(&'static str, StyleDiff, &dyn Debug, &dyn Debug) + 'a;

/// Calls `f(field, class, old, new)` for every field whose value differs. `node` and `tree_order`
/// are skipped (they identify the element / the resolve order, not its style, R60 4.5).
pub(crate) fn style_walk(old: &ComputedStyle, new: &ComputedStyle, f: &mut FieldFn) {
    use StyleDiff::{Box as B, Layout as L, Offset as O, Paint as P};
    // Destructures `old` WITHOUT `..`; `new` is read through field access, so the one exhaustive
    // pattern is what makes a new field a compile error. A class may be an expression over the
    // field's old value (named like the field) and `new`.
    macro_rules! classify {
        (ignore [$($ig:ident),*]; $($fld:ident => $class:expr),* $(,)?) => {{
            let ComputedStyle { $($ig: _,)* $($fld,)* } = old;
            $( if *$fld != new.$fld { f(stringify!($fld), $class, $fld, &new.$fld); } )*
        }};
    }
    classify! {
        ignore [node, tree_order];
        pseudo => B,
        // Custom properties change nothing of this element's own output (the resolved values are
        // in the other fields); children are re-cascaded through the inherited comparison. Paint
        // keeps the cheapest non-None class so a change is never silently dropped.
        custom_props => P,
        color => P,
        background_color => P,
        display => B,
        position => B,
        inset => O,
        // Stacking boundaries change hoisting (R60 6.3): any z-index change is Layout.
        z_index => L,
        // opacity 1 <-> < 1 creates/removes a stacking boundary (Layout); 0.5 -> 0.6 only paints.
        opacity => if (*opacity == 1.0) != (new.opacity == 1.0) { L } else { P },
        translate => O,
        float => B,
        clear => L,
        width => L,
        width_kw => L,
        min_width_kw => L,
        height => L,
        min_width => L,
        max_width => L,
        max_width_intrinsic => L,
        min_height => L,
        max_height => L,
        box_sizing => L,
        list_item => B,
        flow_root => B,
        table_caption => B,
        caption_bottom => B,
        col_span => B,
        row_span => B,
        margin => L,
        margin_auto => L,
        margin_auto_v => L,
        padding => L,
        border_width => L,
        border_color => P,
        border_style => P,
        font_size => L,
        root_font_size => L,
        font_weight => L,
        font_style => L,
        font_family => L,
        text_decoration => P,
        text_decoration_line => P,
        text_decoration_style => P,
        text_decoration_color => P,
        white_space => L,
        word_break => L,
        text_transform => L,
        list_style_type => B,
        text_align => L,
        vertical_align => L,
        line_height => L,
        visibility => P,
        // QCSS-A: an inline svg is rasterised when its box is built.
        svg => B,
        word_spacing => L,
        border_collapse => B,
        text_indent => L,
        outline_width => P,
        outline_color => P,
        box_shadow => P,
        // QCSS-D: pure paint properties (no layout input), so a change repaints without relayout.
        filter => P,
        backdrop_filter => P,
        text_shadow => P,
        clip_path => P,
        mix_blend_mode => P,
        // Background images are fetched/attached by the box builder, so a layout change is a Box change.
        bg_layout => B,
        canvas_src => B,
        viewport_hidden => B,
        viewport_hidden_x => B,
        mask_layout => B,
        object_fit => P,
        aspect_ratio => L,
        object_pos => P,
        border_radius => P,
        overflow_x => B,
        overflow_y => B,
        text_overflow => L,
        animations => L,
        // QANIMHLb: the lists alone change no output (a started transition changes the animated fields).
        transitions => P,
        appearance_none => B,
        contain_lp => L,
        check_widget => B,
        text_widget => B,
        select_widget => B,
        icb_height => L,
        first_line => L,
        cursor => P,
        flex_direction => L,
        flex_wrap => L,
        justify_content => L,
        align_content => L,
        align_items => L,
        align_self => L,
        justify_items => L,
        justify_self => L,
        grid_auto_flow => L,
        flex_grow => L,
        flex_shrink => L,
        flex_basis => L,
        order => B,
        row_gap => L,
        column_gap => L,
        grid_template_columns => L,
        grid_template_rows => L,
        grid_auto_columns => L,
        grid_auto_rows => L,
        grid_template_areas => L,
        grid_area => L,
        grid_row_start => L,
        grid_row_end => L,
        grid_column_start => L,
        grid_column_end => L,
    }
}

/// R60 6.3: the strongest class over every changed field (`None` when the styles are identical).
pub(crate) fn style_diff(old: &ComputedStyle, new: &ComputedStyle) -> StyleDiff {
    let mut d = StyleDiff::None;
    style_walk(old, new, &mut |_, c, _, _| d = d.max(c));
    d
}

/// Every changed field with both values printed (reports; allocates).
pub(crate) fn style_changes(old: &ComputedStyle, new: &ComputedStyle) -> Vec<FieldChange> {
    let mut v = Vec::new();
    style_walk(old, new, &mut |field, class, o, n| v.push(FieldChange { field, class, old: format!("{o:?}"), new: format!("{n:?}") }));
    v
}
