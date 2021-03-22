use core::fmt;
use alloc::collections::btree_map::BTreeMap;
use alloc::vec::Vec;
use azul_css::{
    LayoutRect, LayoutRectVec, LayoutPoint, LayoutSize, PixelValue, StyleFontSize,
    StyleTextColor, ColorU as StyleColorU, OptionF32,
    StyleTextAlignmentHorz, StyleTextAlignmentVert, LayoutPosition,
    CssPropertyValue, LayoutMarginTop, LayoutMarginRight, LayoutMarginLeft, LayoutMarginBottom,
    LayoutPaddingTop, LayoutPaddingLeft, LayoutPaddingRight, LayoutPaddingBottom,
    LayoutLeft, LayoutRight, LayoutTop, LayoutBottom, LayoutFlexDirection, LayoutJustifyContent,
    StyleTransform, StyleTransformOrigin,
};
use crate::{
    styled_dom::{StyledDom, AzNodeId, DomId},
    app_resources::{Words, ShapedWords, FontInstanceKey, WordPositions},
    id_tree::{NodeId, NodeDataContainer},
    dom::{DomNodeHash, ScrollTagId},
    callbacks::{PipelineId, HitTestItem, ScrollHitTestItem},
    window::{ScrollStates, LogicalPosition, LogicalRect, LogicalSize},
};

pub const DEFAULT_FONT_SIZE_PX: isize = 16;
pub const DEFAULT_FONT_SIZE: StyleFontSize = StyleFontSize { inner: PixelValue::const_px(DEFAULT_FONT_SIZE_PX) };
pub const DEFAULT_FONT_ID: &str = "serif";
pub const DEFAULT_TEXT_COLOR: StyleTextColor = StyleTextColor { inner: StyleColorU { r: 0, b: 0, g: 0, a: 255 } };
pub const DEFAULT_LINE_HEIGHT: f32 = 1.0;
pub const DEFAULT_WORD_SPACING: f32 = 1.0;
pub const DEFAULT_LETTER_SPACING: f32 = 0.0;
pub const DEFAULT_TAB_WIDTH: f32 = 4.0;

#[derive(Debug, Clone, PartialEq, PartialOrd)]
#[repr(C)]
pub struct InlineTextLayout {
    pub lines: InlineTextLineVec,
}

impl_vec!(InlineTextLayout, InlineTextLayoutVec, InlineTextLayoutVecDestructor);
impl_vec_clone!(InlineTextLayout, InlineTextLayoutVec, InlineTextLayoutVecDestructor);
impl_vec_debug!(InlineTextLayout, InlineTextLayoutVec);
impl_vec_partialeq!(InlineTextLayout, InlineTextLayoutVec);
impl_vec_partialord!(InlineTextLayout, InlineTextLayoutVec);

/// NOTE: The bounds of the text line is the TOP left corner (relative to the text origin),
/// but the word_position is the BOTTOM left corner (relative to the text line)
#[derive(Debug, Clone, PartialEq, PartialOrd)]
#[repr(C)]
pub struct InlineTextLine {
    pub bounds: LogicalRect,
    /// At which word does this line start?
    pub word_start: usize,
    /// At which word does this line end
    pub word_end: usize,
}

impl_vec!(InlineTextLine, InlineTextLineVec, InlineTextLineVecDestructor);
impl_vec_clone!(InlineTextLine, InlineTextLineVec, InlineTextLineVecDestructor);
impl_vec_mut!(InlineTextLine, InlineTextLineVec);
impl_vec_debug!(InlineTextLine, InlineTextLineVec);
impl_vec_partialeq!(InlineTextLine, InlineTextLineVec);
impl_vec_partialord!(InlineTextLine, InlineTextLineVec);

impl InlineTextLine {
    pub const fn new(bounds: LogicalRect, word_start: usize, word_end: usize) -> Self {
        Self { bounds, word_start, word_end }
    }
}

impl InlineTextLayout {

    #[inline]
    pub fn get_leading(&self) -> f32 {
        match self.lines.as_ref().first() {
            None => 0.0,
            Some(s) => s.bounds.origin.x as f32,
        }
    }

    #[inline]
    pub fn get_trailing(&self) -> f32 {
        match self.lines.as_ref().first() {
            None => 0.0,
            Some(s) => (s.bounds.origin.x + s.bounds.size.width) as f32,
        }
    }

    #[inline]
    pub fn new(lines: Vec<InlineTextLine>) -> Self {
        Self { lines: lines.into() }
    }

    #[inline]
    #[must_use = "get_bounds calls union(self.lines) and is expensive to call"]
    pub fn get_bounds(&self) -> Option<LayoutRect> {
        // because of sub-pixel text positioning, calculating the bound has to be done using floating point
        LogicalRect::union(self.lines.as_ref().iter().map(|c| c.bounds)).map(|s| {
            LayoutRect {
                origin: LayoutPoint::new(libm::floorf(s.origin.x) as isize, libm::floorf(s.origin.y) as isize),
                size: LayoutSize::new(libm::ceilf(s.size.width) as isize, libm::ceilf(s.size.height) as isize),
            }
        })
    }

    #[must_use = "function is expensive to call since it iterates + collects over self.lines"]
    pub fn get_children_horizontal_diff_to_right_edge(&self, parent: &LayoutRect) -> Vec<f32> {
        let parent_right_edge = (parent.origin.x + parent.size.width) as f32;
        let parent_left_edge = parent.origin.x as f32;
        self.lines.as_ref().iter().map(|line| {
            let child_right_edge = line.bounds.origin.x + line.bounds.size.width;
            let child_left_edge = line.bounds.origin.x;
            ((child_left_edge - parent_left_edge) + (parent_right_edge - child_right_edge)) as f32
        }).collect()
    }

    /// Align the lines horizontal to *their bounding box*
    pub fn align_children_horizontal(&mut self, horizontal_alignment: StyleTextAlignmentHorz) {
        let shift_multiplier = match calculate_horizontal_shift_multiplier(horizontal_alignment) {
            None =>  return,
            Some(s) => s,
        };
        let self_bounds = match self.get_bounds() { Some(s) => s, None => { return; }, };
        let horz_diff = self.get_children_horizontal_diff_to_right_edge(&self_bounds);

        for (line, shift) in self.lines.as_mut().iter_mut().zip(horz_diff.into_iter()) {
            line.bounds.origin.x += shift * shift_multiplier;
        }
    }

    /// Align the lines vertical to *their parents container*
    pub fn align_children_vertical_in_parent_bounds(&mut self, parent_size: &LogicalSize, vertical_alignment: StyleTextAlignmentVert) {

        let shift_multiplier = match calculate_vertical_shift_multiplier(vertical_alignment) {
            None =>  return,
            Some(s) => s,
        };

        let self_bounds = match self.get_bounds() { Some(s) => s, None => { return; }, };
        let child_bottom_edge = (self_bounds.origin.y + self_bounds.size.height) as f32;
        let child_top_edge = self_bounds.origin.y as f32;
        let shift = child_top_edge + (parent_size.height - child_bottom_edge);

        for line in self.lines.as_mut().iter_mut() {
            line.bounds.origin.y += shift * shift_multiplier;
        }
    }
}

#[inline]
pub fn calculate_horizontal_shift_multiplier(horizontal_alignment: StyleTextAlignmentHorz) -> Option<f32> {
    use azul_css::StyleTextAlignmentHorz::*;
    match horizontal_alignment {
        Left => None,
        Center => Some(0.5), // move the line by the half width
        Right => Some(1.0), // move the line by the full width
    }
}

#[inline]
pub fn calculate_vertical_shift_multiplier(vertical_alignment: StyleTextAlignmentVert) -> Option<f32> {
    use azul_css::StyleTextAlignmentVert::*;
    match vertical_alignment {
        Top => None,
        Center => Some(0.5), // move the line by the half width
        Bottom => Some(1.0), // move the line by the full width
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq, Ord, PartialOrd)]
#[repr(C)]
pub struct ExternalScrollId(pub u64, pub PipelineId);

impl ::core::fmt::Display for ExternalScrollId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ExternalScrollId({:0x}, {})", self.0, self.1)
    }
}

impl ::core::fmt::Debug for ExternalScrollId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self)
    }
}

#[derive(Debug, Default, Clone, PartialEq, PartialOrd)]
pub struct ScrolledNodes {
    pub overflowing_nodes: BTreeMap<AzNodeId, OverflowingScrollNode>,
    pub tags_to_node_ids: BTreeMap<ScrollTagId, AzNodeId>,
}

#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub struct OverflowingScrollNode {
    pub child_rect: LayoutRect,
    pub parent_external_scroll_id: ExternalScrollId,
    pub parent_dom_hash: DomNodeHash,
    pub scroll_tag_id: ScrollTagId,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum WhConstraint {
    /// between min, max
    Between(f32, f32),
    /// Value needs to be exactly X
    EqualTo(f32),
    /// Value can be anything
    Unconstrained,
}

impl Default for WhConstraint {
    fn default() -> Self { WhConstraint::Unconstrained }
}

impl WhConstraint {

    /// Returns the minimum value or 0 on `Unconstrained`
    /// (warning: this might not be what you want)
    pub fn min_needed_space(&self) -> Option<f32> {
        use self::WhConstraint::*;
        match self {
            Between(min, _) => Some(*min),
            EqualTo(exact) => Some(*exact),
            Unconstrained => None,
        }
    }

    /// Returns the maximum space until the constraint is violated - returns
    /// `None` if the constraint is unbounded
    pub fn max_available_space(&self) -> Option<f32> {
        use self::WhConstraint::*;
        match self {
            Between(_, max) => { Some(*max) },
            EqualTo(exact) => Some(*exact),
            Unconstrained => None,
        }
    }

    /// Returns if this `WhConstraint` is an `EqualTo` constraint
    pub fn is_fixed_constraint(&self) -> bool {
        use self::WhConstraint::*;
        match self {
            EqualTo(_) => true,
            _ => false,
        }
    }

    // The absolute positioned node might have a max-width constraint, which has a
    // higher precedence than `top, bottom, left, right`.
    pub fn calculate_from_relative_parent(&self, relative_parent_width: f32) -> f32 {
        match self {
            WhConstraint::EqualTo(e) => *e,
            WhConstraint::Between(min, max) => {
                relative_parent_width.max(*min).min(*max)
            },
            WhConstraint::Unconstrained => relative_parent_width,
        }
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct WidthCalculatedRect {
    pub preferred_width: WhConstraint,
    pub margin_right: Option<CssPropertyValue<LayoutMarginRight>>,
    pub margin_left: Option<CssPropertyValue<LayoutMarginLeft>>,
    pub padding_right: Option<CssPropertyValue<LayoutPaddingRight>>,
    pub padding_left: Option<CssPropertyValue<LayoutPaddingLeft>>,
    pub left: Option<CssPropertyValue<LayoutLeft>>,
    pub right: Option<CssPropertyValue<LayoutRight>>,
    pub flex_grow_px: f32,
    pub min_inner_size_px: f32,
}

impl WidthCalculatedRect {
    /// Get the flex basis in the horizontal direction - vertical axis has to be calculated differently
    pub fn get_flex_basis_horizontal(&self, parent_width: f32) -> f32 {
        self.preferred_width.min_needed_space().unwrap_or(0.0) +
        self.margin_left.as_ref().and_then(|p| p.get_property().map(|px| px.inner.to_pixels(parent_width))).unwrap_or(0.0) +
        self.margin_right.as_ref().and_then(|p| p.get_property().map(|px| px.inner.to_pixels(parent_width))).unwrap_or(0.0) +
        self.padding_left.as_ref().and_then(|p| p.get_property().map(|px| px.inner.to_pixels(parent_width))).unwrap_or(0.0) +
        self.padding_right.as_ref().and_then(|p| p.get_property().map(|px| px.inner.to_pixels(parent_width))).unwrap_or(0.0)
    }

    /// Get the sum of the horizontal padding amount (`padding.left + padding.right`)
    pub fn get_horizontal_padding(&self, parent_width: f32) -> f32 {
        self.padding_left.as_ref().and_then(|p| p.get_property().map(|px| px.inner.to_pixels(parent_width))).unwrap_or(0.0) +
        self.padding_right.as_ref().and_then(|p| p.get_property().map(|px| px.inner.to_pixels(parent_width))).unwrap_or(0.0)
    }

    /// Called after solver has run: Solved width of rectangle
    pub fn total(&self) -> f32 {
        self.min_inner_size_px + self.flex_grow_px
    }

    pub fn solved_result(&self) -> WidthSolvedResult {
        WidthSolvedResult {
            min_width: self.min_inner_size_px,
            space_added: self.flex_grow_px,
        }
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct HeightCalculatedRect {
    pub preferred_height: WhConstraint,
    pub margin_top: Option<CssPropertyValue<LayoutMarginTop>>,
    pub margin_bottom: Option<CssPropertyValue<LayoutMarginBottom>>,
    pub padding_top: Option<CssPropertyValue<LayoutPaddingTop>>,
    pub padding_bottom: Option<CssPropertyValue<LayoutPaddingBottom>>,
    pub top: Option<CssPropertyValue<LayoutTop>>,
    pub bottom: Option<CssPropertyValue<LayoutBottom>>,
    pub flex_grow_px: f32,
    pub min_inner_size_px: f32,
}

impl HeightCalculatedRect {
    /// Get the flex basis in the horizontal direction - vertical axis has to be calculated differently
    pub fn get_flex_basis_vertical(&self, parent_height: f32) -> f32 {
        let parent_height = parent_height as f32;
        self.preferred_height.min_needed_space().unwrap_or(0.0) +
        self.margin_top.as_ref().and_then(|p| p.get_property().map(|px| px.inner.to_pixels(parent_height))).unwrap_or(0.0) +
        self.margin_bottom.as_ref().and_then(|p| p.get_property().map(|px| px.inner.to_pixels(parent_height))).unwrap_or(0.0) +
        self.padding_top.as_ref().and_then(|p| p.get_property().map(|px| px.inner.to_pixels(parent_height))).unwrap_or(0.0) +
        self.padding_bottom.as_ref().and_then(|p| p.get_property().map(|px| px.inner.to_pixels(parent_height))).unwrap_or(0.0)
    }

    /// Get the sum of the horizontal padding amount (`padding_top + padding_bottom`)
    pub fn get_vertical_padding(&self, parent_height: f32) -> f32 {
        self.padding_top.as_ref().and_then(|p| p.get_property().map(|px| px.inner.to_pixels(parent_height))).unwrap_or(0.0) +
        self.padding_bottom.as_ref().and_then(|p| p.get_property().map(|px| px.inner.to_pixels(parent_height))).unwrap_or(0.0)
    }

    /// Called after solver has run: Solved height of rectangle
    pub fn total(&self) -> f32 {
        self.min_inner_size_px + self.flex_grow_px
    }

    /// Called after solver has run: Solved width of rectangle
    pub fn solved_result(&self) -> HeightSolvedResult {
        HeightSolvedResult {
            min_height: self.min_inner_size_px,
            space_added: self.flex_grow_px,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct WidthSolvedResult {
    pub min_width: f32,
    pub space_added: f32,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct HeightSolvedResult {
    pub min_height: f32,
    pub space_added: f32,
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub struct HorizontalSolvedPosition(pub f32);

#[repr(transparent)]
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub struct VerticalSolvedPosition(pub f32);

#[derive(Debug)]
pub struct LayoutResult {
    pub dom_id: DomId,
    pub parent_dom_id: Option<DomId>,
    pub styled_dom: StyledDom,
    pub root_size: LayoutSize,
    pub root_position: LayoutPoint,
    pub preferred_widths: NodeDataContainer<Option<f32>>,
    pub preferred_heights: NodeDataContainer<Option<f32>>,
    pub width_calculated_rects: NodeDataContainer<WidthCalculatedRect>,
    pub height_calculated_rects: NodeDataContainer<HeightCalculatedRect>,
    pub solved_pos_x: NodeDataContainer<HorizontalSolvedPosition>,
    pub solved_pos_y: NodeDataContainer<VerticalSolvedPosition>,
    pub layout_flex_grows: NodeDataContainer<f32>,
    pub layout_positions: NodeDataContainer<LayoutPosition>,
    pub layout_flex_directions: NodeDataContainer<LayoutFlexDirection>,
    pub layout_justify_contents: NodeDataContainer<LayoutJustifyContent>,
    pub rects: NodeDataContainer<PositionedRectangle>,
    pub words_cache: BTreeMap<NodeId, Words>,
    pub shaped_words_cache: BTreeMap<NodeId, ShapedWords>,
    pub positioned_words_cache: BTreeMap<NodeId, (WordPositions, FontInstanceKey)>,
    pub scrollable_nodes: ScrolledNodes,
    pub iframe_mapping: BTreeMap<NodeId, DomId>,
    pub gpu_value_cache: GpuValueCache,
}

impl LayoutResult {
    pub fn get_bounds(&self) -> LayoutRect { LayoutRect::new(self.root_position, self.root_size) }
}

#[derive(Default, Debug, Clone, PartialEq, PartialOrd)]
pub struct GpuValueCache {
    pub transform_keys: BTreeMap<NodeId, TransformKey>,
    pub current_transform_values: BTreeMap<NodeId, ComputedTransform3D>,
    pub current_opacity_keys: BTreeMap<NodeId, OpacityKey>,
    pub current_opacity_values: BTreeMap<NodeId, f32>,
}

pub enum GpuTransformKeyEvent {
    Added(TransformKey, ComputedTransform3D),
    Changed(TransformKey, ComputedTransform3D),
    Removed(TransformKey),
}

pub enum GpuOpacityKeyEvent {
    Added(OpacityKey, f32),
    Changed(OpacityKey, f32),
    Removed(OpacityKey),
}

pub struct GpuEventChanges {
    pub transform_key_changes: Vec<GpuTransformKeyEvent>,
    pub opacity_key_changes: Vec<OpacityKeyEvent>,
}

pub struct RelayoutChanges {
    pub resized_nodes: Vec<NodeId>,
    pub gpu_key_changes: GpuEventChanges,
}

impl GpuValueCache {

    pub fn empty() -> Self {
        Self::default()
    }

    #[cfg(feature = "multithreading")]
    fn synchronize<'a>(
        &mut self,
        positioned_rects: &NodeDataContainerRef<'a, PositionedRectangle>,
        styled_dom: &StyledDom,
    ) -> GpuEventChanges {

        use rayon::prelude::*;

        let css_property_cache = styled_dom.get_css_property_cache();
        let node_data = styled_dom.node_data.as_container();
        let node_states = styled_dom.styled_nodes.as_container();

        let empty_transform_origin_vec: StyleTransformOriginVec = Vec::new().into();

        // calculate the transform values of every single node
        let all_current_transform_events = (0..styled_dom.len())
        .par_iter()
        .filter_map(|node_id| {
            let node_id = NodeId::new(node_id);
            let transform_origins = css_property_cache.get_transform_origin(node_data[node_id], node_id, node_states[node_id]);
            let current_transform = css_property_cache.get_transform(node_data[node_id], node_id, node_states[node_id]).map(|t| {
                let parent_width = positioned_rects[node_id].total();
                let transform_origins = transform_origins.unwrap_or(&empty_transform_origin_vec);
                ComputedTransform3D::from_style_transform_vec(t.as_ref(), transform_origins, parent_width)
            });
            let existing_transform = self.current_transform_values.get();

            match (existing_transform, current_transform) => {
                (None, None) => None, // no new transform, no old transform
                (None, Some(new)) => Some(GpuTransformKeyEvent::Added(TransformKey::unique(), new)),
                (Some(old), Some(new)) => Some(GpuTransformKeyEvent::Changed(self.transform_keys.get(&node_id).copied()?, old, new)),
                (Some(old), None) => Some(GpuTransformKeyEvent::Removed(self.transform_keys.get(&node_id).copied()?)),
            }
        }).collect();

        let all_current_opacity_events = (0..styled_dom.len())
        .par_iter()
        .filter_map(|node_id| {
            let node_id = NodeId::new(node_id);
            let current_opacity = css_property_cache.get_opacity().unwrap_or_default();
            let existing_opacity = self.current_opacity_values.get();

            match (existing_opacity, current_opacity) => {
                (None, None) => None, // no new opacity, no old transform
                (None, Some(new)) => Some(GpuOpacityKeyEvent::Added(OpacityKey::unique(), new.get())),
                (Some(old), Some(new)) => Some(GpuOpacityKeyEvent::Changed(self.opacity_keys.get(&node_id).copied()?, old, new.get())),
                (Some(old), None) => Some(GpuOpacityKeyEvent::Removed(self.opacity_keys.get(&node_id).copied()?)),
            }
        }).collect();

        // current_transform_values
        // current_opacity_values
        // current_color_values
        /*
            pub transform_keys: BTreeMap<NodeId, TransformKey>,
            pub current_transform_values: BTreeMap<NodeId, ComputedTransform3D>,
            pub opacity_keys: BTreeMap<NodeId, OpacityKey>,
            pub current_opacity_values: BTreeMap<NodeId, f32>,
        */

        GpuEventChanges {
            transform_key_changes: ,
            opacity_key_changes: ,
        }
    }
}

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub struct HitTest {
    pub regular_hit_test_nodes: BTreeMap<NodeId, HitTestItem>,
    pub scroll_hit_test_nodes: BTreeMap<NodeId, ScrollHitTestItem>,
}

impl HitTest {
    pub fn is_empty(&self) -> bool {
        self.regular_hit_test_nodes.is_empty() && self.scroll_hit_test_nodes.is_empty()
    }
}

impl LayoutResult {

    pub fn get_hits(&self, cursor: &LayoutPoint, scroll_states: &ScrollStates) -> HitTest {

        // TODO: SIMD-optimize!

        // insert the regular hit items
        let regular_hit_test_nodes =
        self.styled_dom.tag_ids_to_node_ids
        .as_ref()
        .iter()
        .filter_map(|t| {

            let node_id = t.node_id.into_crate_internal()?;
            let layout_offset = self.rects.as_ref()[node_id].get_static_offset();
            let layout_size = LayoutSize::new(self.width_calculated_rects.as_ref()[node_id].total() as isize, self.height_calculated_rects.as_ref()[node_id].total() as isize);
            let layout_rect = LayoutRect::new(layout_offset, layout_size);

            layout_rect
            .hit_test(cursor)
            .map(|relative_to_item| {
                (node_id, HitTestItem {
                    point_in_viewport: *cursor,
                    point_relative_to_item: relative_to_item,
                    is_iframe_hit: self.iframe_mapping.get(&node_id).map(|iframe_dom_id| {
                        (*iframe_dom_id, layout_offset)
                    }),
                    is_focusable: self.styled_dom.node_data.as_container()[node_id].get_tab_index().into_option().is_some(),
                })
            })
        }).collect();

        // insert the scroll node hit items
        let scroll_hit_test_nodes = self.scrollable_nodes.tags_to_node_ids.iter().filter_map(|(_scroll_tag_id, node_id)| {

            let overflowing_scroll_node = self.scrollable_nodes.overflowing_nodes.get(node_id)?;
            let node_id = node_id.into_crate_internal()?;
            let scroll_state = scroll_states.get_scroll_position(&overflowing_scroll_node.parent_external_scroll_id)?;

            let mut scrolled_cursor = *cursor;
            scrolled_cursor.x += libm::roundf(scroll_state.x) as isize;
            scrolled_cursor.y += libm::roundf(scroll_state.y) as isize;

            let rect = overflowing_scroll_node.child_rect.clone();

            rect.hit_test(&scrolled_cursor).map(|relative_to_scroll| {
                (node_id, ScrollHitTestItem {
                    point_in_viewport: *cursor,
                    point_relative_to_item: relative_to_scroll,
                    scroll_node: overflowing_scroll_node.clone(),
                })
            })
        }).collect();

        HitTest {
            regular_hit_test_nodes,
            scroll_hit_test_nodes,
        }
    }
}

/// Layout options that can impact the flow of word positions
#[derive(Debug, Clone, PartialEq, PartialOrd, Default)]
pub struct TextLayoutOptions {
    /// Font size (in pixels) that this text has been laid out with
    pub font_size_px: PixelValue,
    /// Multiplier for the line height, default to 1.0
    pub line_height: Option<f32>,
    /// Additional spacing between glyphs (in pixels)
    pub letter_spacing: Option<PixelValue>,
    /// Additional spacing between words (in pixels)
    pub word_spacing: Option<PixelValue>,
    /// How many spaces should a tab character emulate
    /// (multiplying value, i.e. `4.0` = one tab = 4 spaces)?
    pub tab_width: Option<f32>,
    /// Maximum width of the text (in pixels) - if the text is set to `overflow:visible`, set this to None.
    pub max_horizontal_width: Option<f32>,
    /// How many pixels of leading does the first line have? Note that this added onto to the holes,
    /// so for effects like `:first-letter`, use a hole instead of a leading.
    pub leading: Option<f32>,
    /// This is more important for inline text layout where items can punch "holes"
    /// into the text flow, for example an image that floats to the right.
    ///
    /// TODO: Currently unused!
    pub holes: Vec<LayoutRect>,
}

/// Same as `TextLayoutOptions`, but with the widths / heights of the `PixelValue`s
/// resolved to regular f32s (because `letter_spacing`, `word_spacing`, etc. may be %-based value)
#[derive(Debug, Clone, PartialEq, PartialOrd, Default)]
pub struct ResolvedTextLayoutOptions {
    /// Font size (in pixels) that this text has been laid out with
    pub font_size_px: f32,
    /// Multiplier for the line height, default to 1.0
    pub line_height: OptionF32,
    /// Additional spacing between glyphs (in pixels)
    pub letter_spacing: OptionF32,
    /// Additional spacing between words (in pixels)
    pub word_spacing: OptionF32,
    /// How many spaces should a tab character emulate
    /// (multiplying value, i.e. `4.0` = one tab = 4 spaces)?
    pub tab_width: OptionF32,
    /// Maximum width of the text (in pixels) - if the text is set to `overflow:visible`, set this to None.
    pub max_horizontal_width: OptionF32,
    /// How many pixels of leading does the first line have? Note that this added onto to the holes,
    /// so for effects like `:first-letter`, use a hole instead of a leading.
    pub leading: OptionF32,
    /// This is more important for inline text layout where items can punch "holes"
    /// into the text flow, for example an image that floats to the right.
    ///
    /// TODO: Currently unused!
    pub holes: LayoutRectVec,
}

#[derive(Debug, Default, Copy, Clone, PartialEq, PartialOrd)]
#[repr(C)]
pub struct ResolvedOffsets {
    pub top: f32,
    pub left: f32,
    pub right: f32,
    pub bottom: f32,
}

impl ResolvedOffsets {
    pub const fn zero() -> Self { Self { top: 0.0, left: 0.0, right: 0.0, bottom: 0.0 } }
    pub fn total_vertical(&self) -> f32 { self.top + self.bottom }
    pub fn total_horizontal(&self) -> f32 { self.left + self.right }
}

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub struct PositionedRectangle {
    /// Outer bounds of the rectangle
    pub size: LogicalSize,
    /// How the rectangle should be positioned
    pub position: PositionInfo,
    /// Padding of the rectangle
    pub padding: ResolvedOffsets,
    /// Margin of the rectangle
    pub margin: ResolvedOffsets,
    /// Border widths of the rectangle
    pub border_widths: ResolvedOffsets,
    // TODO: box_shadow_widths
    /// If this is an inline rectangle, resolve the %-based font sizes
    /// and store them here.
    pub resolved_text_layout_options: Option<(ResolvedTextLayoutOptions, InlineTextLayout)>,
    /// Determines if the rect should be clipped or not (TODO: x / y as separate fields!)
    pub overflow: OverflowInfo,
}

impl Default for PositionedRectangle {
    fn default() -> Self {
        PositionedRectangle {
            size: LogicalSize::zero(),
            position: PositionInfo::Static { x_offset: 0.0, y_offset: 0.0, static_x_offset: 0.0, static_y_offset: 0.0 },
            padding: ResolvedOffsets::zero(),
            margin: ResolvedOffsets::zero(),
            border_widths: ResolvedOffsets::zero(),
            resolved_text_layout_options: None,
            overflow: OverflowInfo::default(),
        }
    }
}

impl PositionedRectangle {

    #[inline]
    pub(crate) fn get_approximate_static_bounds(&self) -> LayoutRect {
        LayoutRect::new(self.get_static_offset(), self.get_content_size())
    }

    // Returns the rect where the content should be placed (for example the text itself)
    #[inline]
    fn get_content_size(&self) -> LayoutSize {
        LayoutSize::new(libm::roundf(self.size.width) as isize, libm::roundf(self.size.height) as isize)
    }

    /// Same as get_logical_relative_offset, but returns the relative offset, not the screen-space static one
    #[inline]
    pub(crate) fn get_logical_relative_offset(&self) -> LogicalPosition {
        match self.position {
            PositionInfo::Static { x_offset, y_offset, .. } |
            PositionInfo::Fixed { x_offset, y_offset, .. } |
            PositionInfo::Absolute { x_offset, y_offset, .. } |
            PositionInfo::Relative { x_offset, y_offset, .. } => {
                LogicalPosition::new(x_offset, y_offset)
            },
        }
    }


    /// Same as get_static_offset, but not rounded
    #[inline]
    pub(crate) fn get_logical_static_offset(&self) -> LogicalPosition {
        match self.position {
            PositionInfo::Static { static_x_offset, static_y_offset, .. } |
            PositionInfo::Fixed { static_x_offset, static_y_offset, .. } |
            PositionInfo::Absolute { static_x_offset, static_y_offset, .. } |
            PositionInfo::Relative { static_x_offset, static_y_offset, .. } => {
                LogicalPosition::new(static_x_offset, static_y_offset)
            },
        }
    }

    #[inline]
    fn get_static_offset(&self) -> LayoutPoint {
        match self.position {
            PositionInfo::Static { static_x_offset, static_y_offset, .. } |
            PositionInfo::Fixed { static_x_offset, static_y_offset, .. } |
            PositionInfo::Absolute { static_x_offset, static_y_offset, .. } |
            PositionInfo::Relative { static_x_offset, static_y_offset, .. } => {
                LayoutPoint::new(libm::roundf(static_x_offset) as isize, libm::roundf(static_y_offset) as isize)
            },
        }
    }

    #[inline]
    pub const fn to_layouted_rectangle(&self) -> LayoutedRectangle {
        LayoutedRectangle {
            size: self.size,
            position: self.position,
            padding: self.padding,
            margin: self.margin,
            border_widths: self.border_widths,
            overflow: self.overflow,
        }
    }

    // Returns the rect that includes bounds, expanded by the padding + the border widths
    #[inline]
    pub fn get_background_bounds(&self) -> (LogicalSize, PositionInfo) {

        use crate::ui_solver::PositionInfo::*;

        let b_size = LogicalSize {
            width: self.size.width + self.padding.total_horizontal() + self.border_widths.total_horizontal(),
            height: self.size.height + self.padding.total_vertical() + self.border_widths.total_vertical(),
        };

        let x_offset_add = 0.0 - self.padding.left - self.border_widths.left;
        let y_offset_add = 0.0 - self.padding.top - self.border_widths.top;

        let b_position = match self.position {
            Static { x_offset, y_offset, static_x_offset, static_y_offset } => Static { x_offset: x_offset + x_offset_add, y_offset: y_offset + y_offset_add, static_x_offset, static_y_offset },
            Fixed { x_offset, y_offset, static_x_offset, static_y_offset } => Fixed { x_offset: x_offset + x_offset_add, y_offset: y_offset + y_offset_add, static_x_offset, static_y_offset },
            Relative { x_offset, y_offset, static_x_offset, static_y_offset } => Relative { x_offset: x_offset + x_offset_add, y_offset: y_offset + y_offset_add, static_x_offset, static_y_offset },
            Absolute { x_offset, y_offset, static_x_offset, static_y_offset } => Absolute { x_offset: x_offset + x_offset_add, y_offset: y_offset + y_offset_add, static_x_offset, static_y_offset },
        };

        (b_size, b_position)
    }

    #[inline]
    pub fn get_margin_box_width(&self) -> f32 {
        self.size.width +
        self.padding.total_horizontal() +
        self.border_widths.total_horizontal() +
        self.margin.total_horizontal()
    }

    #[inline]
    pub fn get_margin_box_height(&self) -> f32 {
        self.size.height +
        self.padding.total_vertical() +
        self.border_widths.total_vertical() +
        self.margin.total_vertical()
    }

    #[inline]
    pub fn get_left_leading(&self) -> f32 {
        self.margin.left +
        self.padding.left +
        self.border_widths.left
    }

    #[inline]
    pub fn get_top_leading(&self) -> f32 {
        self.margin.top +
        self.padding.top +
        self.border_widths.top
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq, PartialOrd)]
pub struct OverflowInfo {
    pub overflow_x: DirectionalOverflowInfo,
    pub overflow_y: DirectionalOverflowInfo,
}

// stores how much the children overflow the parent in the given direction
// if amount is negative, the children do not overflow the parent
// if the amount is set to None, that means there are no children for this node, so no overflow can be calculated
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub enum DirectionalOverflowInfo {
    Scroll { amount: Option<isize> },
    Auto { amount: Option<isize> },
    Hidden { amount: Option<isize> },
    Visible { amount: Option<isize> },
}

impl Default for DirectionalOverflowInfo {
    fn default() -> DirectionalOverflowInfo {
        DirectionalOverflowInfo::Auto { amount: None }
    }
}

impl DirectionalOverflowInfo {

    #[inline]
    pub fn get_amount(&self) -> Option<isize> {
        match self {
            DirectionalOverflowInfo::Scroll { amount: Some(s) } |
            DirectionalOverflowInfo::Auto { amount: Some(s) } |
            DirectionalOverflowInfo::Hidden { amount: Some(s) } |
            DirectionalOverflowInfo::Visible { amount: Some(s) } => Some(*s),
            _ => None
        }
    }

    #[inline]
    pub fn is_negative(&self) -> bool {
        match self {
            DirectionalOverflowInfo::Scroll { amount: Some(s) } |
            DirectionalOverflowInfo::Auto { amount: Some(s) } |
            DirectionalOverflowInfo::Hidden { amount: Some(s) } |
            DirectionalOverflowInfo::Visible { amount: Some(s) } => { *s < 0_isize },
            _ => true // no overflow = no scrollbar
        }
    }

    #[inline]
    pub fn is_none(&self) -> bool {
        match self {
            DirectionalOverflowInfo::Scroll { amount: None } |
            DirectionalOverflowInfo::Auto { amount: None } |
            DirectionalOverflowInfo::Hidden { amount: None } |
            DirectionalOverflowInfo::Visible { amount: None } => true,
            _ => false
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub enum PositionInfo {
    Static { x_offset: f32, y_offset: f32, static_x_offset: f32, static_y_offset: f32 },
    Fixed { x_offset: f32, y_offset: f32, static_x_offset: f32, static_y_offset: f32 },
    Absolute { x_offset: f32, y_offset: f32, static_x_offset: f32, static_y_offset: f32 },
    Relative { x_offset: f32, y_offset: f32, static_x_offset: f32, static_y_offset: f32 },
}

impl PositionInfo {
    #[inline]
    pub fn is_positioned(&self) -> bool {
        match self {
            PositionInfo::Static { .. } => false,
            PositionInfo::Fixed { .. } => true,
            PositionInfo::Absolute { .. } => true,
            PositionInfo::Relative { .. } => true,
        }
    }
    #[inline]
    pub fn get_relative_offset(&self) -> (f32, f32) {
        match self {
            PositionInfo::Static { x_offset, y_offset, .. } |
            PositionInfo::Fixed { x_offset, y_offset, .. } |
            PositionInfo::Absolute { x_offset, y_offset, .. } |
            PositionInfo::Relative { x_offset, y_offset, .. } => (*x_offset, *y_offset)
        }
    }
}

/// Same as `PositionedRectangle`, but without the `text_layout_options`,
/// so that the struct implements `Copy`.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub struct LayoutedRectangle {
    /// Outer bounds of the rectangle
    pub size: LogicalSize,
    /// How the rectangle should be positioned
    pub position: PositionInfo,
    /// Padding of the rectangle
    pub padding: ResolvedOffsets,
    /// Margin of the rectangle
    pub margin: ResolvedOffsets,
    /// Border widths of the rectangle
    pub border_widths: ResolvedOffsets,
    /// Determines if the rect should be clipped or not (TODO: x / y as separate fields!)
    pub overflow: OverflowInfo,
}

/// Computed transform of pixels in pixel space, optimized
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
#[repr(packed)]
pub struct ComputedTransform3D {
    pub m:[[f32;4];4]
}

impl ComputedTransform3D {

    pub const IDENTITY: Self = Self {
        m: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    };

    pub const fn new(
        m11: f32, m12: f32, m13: f32, m14: f32,
        m21: f32, m22: f32, m23: f32, m24: f32,
        m31: f32, m32: f32, m33: f32, m34: f32,
        m41: f32, m42: f32, m43: f32, m44: f32
    ) -> Self {
        Self {
            m: [
                [m11, m12, m13, m14],
                [m21, m22, m23, m24],
                [m31, m32, m33, m34],
                [m41, m42, m43, m44],
            ]
        }
    }

    pub const fn new_2d(
        m11: f32, m12: f32,
        m21: f32, m22: f32,
        m41: f32, m42: f32
    ) -> Self {
         Self::new(
             m11,  m12, 0.0, 0.0,
             m21,  m22, 0.0, 0.0,
             0.0,  0.0, 1.0, 0.0,
             m41,  m42, 0.0, 1.0
        )
    }

    // Computes the matrix of a rect from a Vec<StyleTransform>
    pub fn from_style_transform_vec(t_vec: &[StyleTransform], transform_origins: &[StyleTransformOrigin], percent_resolve: f32) -> Self {

        // TODO: use correct SIMD optimization!
        let mut matrix = Self::IDENTITY;
        let default_origin = StyleTransformOrigin::default();

        for (t_idx, t) in t_vec.iter().enumerate() {
            let transform_origin = transform_origins.get(t_idx).unwrap_or(transform_origins.get(0).unwrap_or(&default_origin));
            matrix.then(Self::from_style_transform(t, transform_origin, percent_resolve));
        }

        matrix
    }

    /// Creates a new transform from a style transform using the
    /// parent width as a way to resolve for percentages
    pub fn from_style_transform(t: &StyleTransform, transform_origin: &StyleTransformOrigin, percent_resolve: f32) -> Self {
        use azul_css::StyleTransform::*;
        match t {
            Matrix(mat2d) => {
                let a = mat2d.a.to_pixels(percent_resolve);
                let b = mat2d.b.to_pixels(percent_resolve);
                let c = mat2d.c.to_pixels(percent_resolve);
                let d = mat2d.d.to_pixels(percent_resolve);
                let tx = mat2d.tx.to_pixels(percent_resolve);
                let ty = mat2d.ty.to_pixels(percent_resolve);

                Self::new_2d(a, b, c, d, tx, ty)
            },
            Matrix3D(mat3d) => {
                let m11 = mat3d.m11.to_pixels(percent_resolve);
                let m12 = mat3d.m12.to_pixels(percent_resolve);
                let m13 = mat3d.m13.to_pixels(percent_resolve);
                let m14 = mat3d.m14.to_pixels(percent_resolve);
                let m21 = mat3d.m21.to_pixels(percent_resolve);
                let m22 = mat3d.m22.to_pixels(percent_resolve);
                let m23 = mat3d.m23.to_pixels(percent_resolve);
                let m24 = mat3d.m24.to_pixels(percent_resolve);
                let m31 = mat3d.m31.to_pixels(percent_resolve);
                let m32 = mat3d.m32.to_pixels(percent_resolve);
                let m33 = mat3d.m33.to_pixels(percent_resolve);
                let m34 = mat3d.m34.to_pixels(percent_resolve);
                let m41 = mat3d.m41.to_pixels(percent_resolve);
                let m42 = mat3d.m42.to_pixels(percent_resolve);
                let m43 = mat3d.m43.to_pixels(percent_resolve);
                let m44 = mat3d.m44.to_pixels(percent_resolve);

                Self::new(
                    m11,
                    m12,
                    m13,
                    m14,
                    m21,
                    m22,
                    m23,
                    m24,
                    m31,
                    m32,
                    m33,
                    m34,
                    m41,
                    m42,
                    m43,
                    m44,
                )
            },
            Translate(trans2d) => Self::new_translation(
                trans2d.x.to_pixels(percent_resolve),
                trans2d.y.to_pixels(percent_resolve),
                0.0
            ),
            Translate3D(trans3d) => Self::new_translation(
                trans3d.x.to_pixels(percent_resolve),
                trans3d.y.to_pixels(percent_resolve),
                trans3d.z.to_pixels(percent_resolve)
            ),
            TranslateX(trans_x) => Self::new_translation(trans_x.to_pixels(percent_resolve), 0.0, 0.0),
            TranslateY(trans_y) => Self::new_translation(0.0, trans_y.to_pixels(percent_resolve), 0.0),
            TranslateZ(trans_z) => Self::new_translation(0.0, 0.0, trans_z.to_pixels(percent_resolve)),
            Rotate3D(rot3d) => {
                let rotation_origin = (transform_origin.x.to_pixels(percent_resolve), transform_origin.y.to_pixels(percent_resolve));
                Self::make_rotation(
                    rotation_origin,
                    rot3d.angle.to_degrees(),
                    rot3d.x.normalized(),
                    rot3d.y.normalized(),
                    rot3d.z.normalized(),
                )
            },
            RotateX(angle_x) => {
                let rotation_origin = (transform_origin.x.to_pixels(percent_resolve), transform_origin.y.to_pixels(percent_resolve));
                Self::make_rotation(
                    rotation_origin,
                    angle_x.to_degrees(),
                    1.0,
                    0.0,
                    0.0,
                )
            },
            RotateY(angle_y) => {
                let rotation_origin = (transform_origin.x.to_pixels(percent_resolve), transform_origin.y.to_pixels(percent_resolve));
                Self::make_rotation(
                    rotation_origin,
                    angle_y.to_degrees(),
                    0.0,
                    1.0,
                    0.0,
                )
            },
            Rotate(angle_z) | RotateZ(angle_z) => {
                let rotation_origin = (transform_origin.x.to_pixels(percent_resolve), transform_origin.y.to_pixels(percent_resolve));
                Self::make_rotation(
                    rotation_origin,
                    angle_z.to_degrees(),
                    0.0,
                    0.0,
                    1.0,
                )
            },
            Scale(scale2d) => Self::new_scale(
                scale2d.x.normalized(),
                scale2d.y.normalized(),
                0.0,
            ),
            Scale3D(scale3d) => Self::new_scale(
                scale3d.x.normalized(),
                scale3d.y.normalized(),
                scale3d.z.normalized(),
            ),
            ScaleX(scale_x) => Self::new_scale(scale_x.normalized(), 0.0, 0.0),
            ScaleY(scale_y) => Self::new_scale(0.0, scale_y.normalized(), 0.0),
            ScaleZ(scale_z) => Self::new_scale(0.0, 0.0, scale_z.normalized()),
            Skew(skew2d) => Self::new_skew(skew2d.x.normalized(), skew2d.y.normalized()),
            SkewX(skew_x) => Self::new_skew(skew_x.normalized(), 0.0),
            SkewY(skew_y) => Self::new_skew(0.0, skew_y.normalized()),
            Perspective(px) => Self::new_perspective(px.to_pixels(percent_resolve)),
        }
    }

    #[inline]
    pub const fn new_scale(x: f32, y: f32, z: f32) -> Self {
        Self::new(
            x,   0.0, 0.0, 0.0,
            0.0, y,   0.0, 0.0,
            0.0, 0.0, z,   0.0,
            0.0, 0.0, 0.0, 1.0,
        )
    }

    #[inline]
    pub const fn new_translation(x: f32, y: f32, z: f32) -> Self {
        Self::new(
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
             x,  y,   z,   1.0,
        )
    }

    #[inline]
    pub fn new_perspective(d: f32) -> Self {
        Self::new(
            1.0, 0.0, 0.0,  0.0,
            0.0, 1.0, 0.0,  0.0,
            0.0, 0.0, 1.0, -1.0 / d,
            0.0, 0.0, 0.0,  1.0,
        )
    }

    /// Create a 3d rotation transform from an angle / axis.
    /// The supplied axis must be normalized.
    #[inline]
    pub fn new_rotation(x: f32, y: f32, z: f32, theta: f32) -> Self {

        let xx = x * x;
        let yy = y * y;
        let zz = z * z;

        let half_theta = theta / 2.0;
        let sc = half_theta.sin() * half_theta.cos();
        let sq = half_theta.sin() * half_theta.sin();

        Self::new(
            1.0 - 2.0 * (yy + zz) * sq,
            2.0 * (x * y * sq + z * sc),
            2.0 * (x * z * sq - y * sc),
            0.0,


            2.0 * (x * y * sq - z * sc),
            1.0 - 2.0 * (xx + zz) * sq,
            2.0 * (y * z * sq + x * sc),
            0.0,

            2.0 * (x * z * sq + y * sc),
            2.0 * (y * z * sq - x * sc),
            1.0 - 2.0 * (xx + yy) * sq,
            0.0,

            0.0,
            0.0,
            0.0,
            1.0
        )
    }

    #[inline]
    pub fn new_skew(alpha: f32, beta: f32) -> Self {
        let (sx, sy) = (beta.to_radians().tan(), alpha.to_radians().tan());
        Self::new(
            1.0, sx,  0.0, 0.0,
            sy,  1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        )
    }

    // Transforms a 2D point into the target coordinate space
    #[must_use]
    pub fn transform_point2d(&self, point: LogicalPosition) -> Option<LogicalPosition> {
        let w = p.x.mul_add(self.m[0][3], p.y.mul_add(self.m[1][3], self.m[3][3]);

        if !w.is_sign_positive() { None }

        let x = p.x.mul_add(self.m[0][0], p.y.mul_add(self.m[1][0], self.m[3][0]);
        let y = p.x.mul_add(self.m[0][1], p.y.mul_add(self.m[1][1], self.m[3][1]);

        Some(LogicalPosition { x: x / w, y: y / w }
    }

    /// Computes the sum of two matrices while applying `other` AFTER the current matrix.
    #[must_use]
    pub fn then(&self, other: &Self) -> Self {
        Self::new(
            self.m[0][0].mul_add(other.m[0][0], self.m[0][1].mul_add(other.m[1][0], self.m[0][2].mul_add(other.m[2][0], self.m[0][3] * other.m[3][0]))),
            self.m[0][0].mul_add(other.m[0][1], self.m[0][1].mul_add(other.m[1][1], self.m[0][2].mul_add(other.m[2][1], self.m[0][3] * other.m[3][1]))),
            self.m[0][0].mul_add(other.m[0][2], self.m[0][1].mul_add(other.m[1][2], self.m[0][2].mul_add(other.m[3][2], self.m[0][3] * other.m[3][2]))),
            self.m[0][0].mul_add(other.m[0][3], self.m[0][1].mul_add(other.m[1][3], self.m[0][2].mul_add(other.m[2][3], self.m[0][3] * other.m[3][3]))),
            self.m[1][0].mul_add(other.m[0][0], self.m[1][1].mul_add(other.m[1][0], self.m[1][2].mul_add(other.m[2][0], self.m[1][3] * other.m[3][0]))),
            self.m[1][0].mul_add(other.m[0][1], self.m[1][1].mul_add(other.m[1][1], self.m[1][2].mul_add(other.m[2][1], self.m[1][3] * other.m[3][1]))),
            self.m[1][0].mul_add(other.m[0][2], self.m[1][1].mul_add(other.m[1][2], self.m[1][2].mul_add(other.m[3][2], self.m[1][3] * other.m[3][2]))),
            self.m[1][0].mul_add(other.m[0][3], self.m[1][1].mul_add(other.m[1][3], self.m[1][2].mul_add(other.m[2][3], self.m[1][3] * other.m[3][3]))),
            self.m[2][0].mul_add(other.m[0][0], self.m[2][1].mul_add(other.m[1][0], self.m[3][2].mul_add(other.m[2][0], self.m[2][3] * other.m[3][0]))),
            self.m[2][0].mul_add(other.m[0][1], self.m[2][1].mul_add(other.m[1][1], self.m[3][2].mul_add(other.m[2][1], self.m[2][3] * other.m[3][1]))),
            self.m[2][0].mul_add(other.m[0][2], self.m[2][1].mul_add(other.m[1][2], self.m[3][2].mul_add(other.m[3][2], self.m[2][3] * other.m[3][2]))),
            self.m[2][0].mul_add(other.m[0][3], self.m[2][1].mul_add(other.m[1][3], self.m[3][2].mul_add(other.m[2][3], self.m[2][3] * other.m[3][3]))),
            self.m[3][0].mul_add(other.m[0][0], self.m[3][1].mul_add(other.m[1][0], self.m[3][2].mul_add(other.m[2][0], self.m[3][3] * other.m[3][0]))),
            self.m[3][0].mul_add(other.m[0][1], self.m[3][1].mul_add(other.m[1][1], self.m[3][2].mul_add(other.m[2][1], self.m[3][3] * other.m[3][1]))),
            self.m[3][0].mul_add(other.m[0][2], self.m[3][1].mul_add(other.m[1][2], self.m[3][2].mul_add(other.m[3][2], self.m[3][3] * other.m[3][2]))),
            self.m[3][0].mul_add(other.m[0][3], self.m[3][1].mul_add(other.m[1][3], self.m[3][2].mul_add(other.m[2][3], self.m[3][3] * other.m[3][3]))),
        )
    }

    /// Computes the inverse of the matrix, returns None if the determinant is zero.
    #[must_use]
    pub fn inverse(&self) -> Option<Self> {
        let det = self.determinant();

        if det == 0.0 {
            return None;
        }

        // todo(gw): this could be made faster by special casing
        // for simpler transform types.
        let m = Self::new(
            self.m[1][2]*self.m[2][3]*self.m[3][1] - self.m[1][3]*self.m[3][2]*self.m[3][1] +
            self.m[1][3]*self.m[2][1]*self.m[3][2] - self.m[1][1]*self.m[2][3]*self.m[3][2] -
            self.m[1][2]*self.m[2][1]*self.m[3][3] + self.m[1][1]*self.m[3][2]*self.m[3][3],

            self.m[0][3]*self.m[3][2]*self.m[3][1] - self.m[0][2]*self.m[2][3]*self.m[3][1] -
            self.m[0][3]*self.m[2][1]*self.m[3][2] + self.m[0][1]*self.m[2][3]*self.m[3][2] +
            self.m[0][2]*self.m[2][1]*self.m[3][3] - self.m[0][1]*self.m[3][2]*self.m[3][3],

            self.m[0][2]*self.m[1][3]*self.m[3][1] - self.m[0][3]*self.m[1][2]*self.m[3][1] +
            self.m[0][3]*self.m[1][1]*self.m[3][2] - self.m[0][1]*self.m[1][3]*self.m[3][2] -
            self.m[0][2]*self.m[1][1]*self.m[3][3] + self.m[0][1]*self.m[1][2]*self.m[3][3],

            self.m[0][3]*self.m[1][2]*self.m[2][1] - self.m[0][2]*self.m[1][3]*self.m[2][1] -
            self.m[0][3]*self.m[1][1]*self.m[3][2] + self.m[0][1]*self.m[1][3]*self.m[3][2] +
            self.m[0][2]*self.m[1][1]*self.m[2][3] - self.m[0][1]*self.m[1][2]*self.m[2][3],

            self.m[1][3]*self.m[3][2]*self.m[3][0] - self.m[1][2]*self.m[2][3]*self.m[3][0] -
            self.m[1][3]*self.m[2][0]*self.m[3][2] + self.m[1][0]*self.m[2][3]*self.m[3][2] +
            self.m[1][2]*self.m[2][0]*self.m[3][3] - self.m[1][0]*self.m[3][2]*self.m[3][3],

            self.m[0][2]*self.m[2][3]*self.m[3][0] - self.m[0][3]*self.m[3][2]*self.m[3][0] +
            self.m[0][3]*self.m[2][0]*self.m[3][2] - self.m[0][0]*self.m[2][3]*self.m[3][2] -
            self.m[0][2]*self.m[2][0]*self.m[3][3] + self.m[0][0]*self.m[3][2]*self.m[3][3],

            self.m[0][3]*self.m[1][2]*self.m[3][0] - self.m[0][2]*self.m[1][3]*self.m[3][0] -
            self.m[0][3]*self.m[1][0]*self.m[3][2] + self.m[0][0]*self.m[1][3]*self.m[3][2] +
            self.m[0][2]*self.m[1][0]*self.m[3][3] - self.m[0][0]*self.m[1][2]*self.m[3][3],

            self.m[0][2]*self.m[1][3]*self.m[2][0] - self.m[0][3]*self.m[1][2]*self.m[2][0] +
            self.m[0][3]*self.m[1][0]*self.m[3][2] - self.m[0][0]*self.m[1][3]*self.m[3][2] -
            self.m[0][2]*self.m[1][0]*self.m[2][3] + self.m[0][0]*self.m[1][2]*self.m[2][3],

            self.m[1][1]*self.m[2][3]*self.m[3][0] - self.m[1][3]*self.m[2][1]*self.m[3][0] +
            self.m[1][3]*self.m[2][0]*self.m[3][1] - self.m[1][0]*self.m[2][3]*self.m[3][1] -
            self.m[1][1]*self.m[2][0]*self.m[3][3] + self.m[1][0]*self.m[2][1]*self.m[3][3],

            self.m[0][3]*self.m[2][1]*self.m[3][0] - self.m[0][1]*self.m[2][3]*self.m[3][0] -
            self.m[0][3]*self.m[2][0]*self.m[3][1] + self.m[0][0]*self.m[2][3]*self.m[3][1] +
            self.m[0][1]*self.m[2][0]*self.m[3][3] - self.m[0][0]*self.m[2][1]*self.m[3][3],

            self.m[0][1]*self.m[1][3]*self.m[3][0] - self.m[0][3]*self.m[1][1]*self.m[3][0] +
            self.m[0][3]*self.m[1][0]*self.m[3][1] - self.m[0][0]*self.m[1][3]*self.m[3][1] -
            self.m[0][1]*self.m[1][0]*self.m[3][3] + self.m[0][0]*self.m[1][1]*self.m[3][3],

            self.m[0][3]*self.m[1][1]*self.m[2][0] - self.m[0][1]*self.m[1][3]*self.m[2][0] -
            self.m[0][3]*self.m[1][0]*self.m[2][1] + self.m[0][0]*self.m[1][3]*self.m[2][1] +
            self.m[0][1]*self.m[1][0]*self.m[2][3] - self.m[0][0]*self.m[1][1]*self.m[2][3],

            self.m[1][2]*self.m[2][1]*self.m[3][0] - self.m[1][1]*self.m[3][2]*self.m[3][0] -
            self.m[1][2]*self.m[2][0]*self.m[3][1] + self.m[1][0]*self.m[3][2]*self.m[3][1] +
            self.m[1][1]*self.m[2][0]*self.m[3][2] - self.m[1][0]*self.m[2][1]*self.m[3][2],

            self.m[0][1]*self.m[3][2]*self.m[3][0] - self.m[0][2]*self.m[2][1]*self.m[3][0] +
            self.m[0][2]*self.m[2][0]*self.m[3][1] - self.m[0][0]*self.m[3][2]*self.m[3][1] -
            self.m[0][1]*self.m[2][0]*self.m[3][2] + self.m[0][0]*self.m[2][1]*self.m[3][2],

            self.m[0][2]*self.m[1][1]*self.m[3][0] - self.m[0][1]*self.m[1][2]*self.m[3][0] -
            self.m[0][2]*self.m[1][0]*self.m[3][1] + self.m[0][0]*self.m[1][2]*self.m[3][1] +
            self.m[0][1]*self.m[1][0]*self.m[3][2] - self.m[0][0]*self.m[1][1]*self.m[3][2],

            self.m[0][1]*self.m[1][2]*self.m[2][0] - self.m[0][2]*self.m[1][1]*self.m[2][0] +
            self.m[0][2]*self.m[1][0]*self.m[2][1] - self.m[0][0]*self.m[1][2]*self.m[2][1] -
            self.m[0][1]*self.m[1][0]*self.m[3][2] + self.m[0][0]*self.m[1][1]*self.m[3][2]
        );

        Some(m.multiply_scalar(1.0 / det))
    }

    /// Compute the determinant of the transform.
    #[inline]
    pub fn determinant(&self) -> f32 {
        // TODO: SIMD
        self.m[0][3] * self.m[1][2] * self.m[2][1] * self.m[3][0] -
        self.m[0][2] * self.m[1][3] * self.m[2][1] * self.m[3][0] -
        self.m[0][3] * self.m[1][1] * self.m[3][2] * self.m[3][0] +
        self.m[0][1] * self.m[1][3] * self.m[3][2] * self.m[3][0] +
        self.m[0][2] * self.m[1][1] * self.m[2][3] * self.m[3][0] -
        self.m[0][1] * self.m[1][2] * self.m[2][3] * self.m[3][0] -
        self.m[0][3] * self.m[1][2] * self.m[2][0] * self.m[3][1] +
        self.m[0][2] * self.m[1][3] * self.m[2][0] * self.m[3][1] +
        self.m[0][3] * self.m[1][0] * self.m[3][2] * self.m[3][1] -
        self.m[0][0] * self.m[1][3] * self.m[3][2] * self.m[3][1] -
        self.m[0][2] * self.m[1][0] * self.m[2][3] * self.m[3][1] +
        self.m[0][0] * self.m[1][2] * self.m[2][3] * self.m[3][1] +
        self.m[0][3] * self.m[1][1] * self.m[2][0] * self.m[3][2] -
        self.m[0][1] * self.m[1][3] * self.m[2][0] * self.m[3][2] -
        self.m[0][3] * self.m[1][0] * self.m[2][1] * self.m[3][2] +
        self.m[0][0] * self.m[1][3] * self.m[2][1] * self.m[3][2] +
        self.m[0][1] * self.m[1][0] * self.m[2][3] * self.m[3][2] -
        self.m[0][0] * self.m[1][1] * self.m[2][3] * self.m[3][2] -
        self.m[0][2] * self.m[1][1] * self.m[2][0] * self.m[3][3] +
        self.m[0][1] * self.m[1][2] * self.m[2][0] * self.m[3][3] +
        self.m[0][2] * self.m[1][0] * self.m[2][1] * self.m[3][3] -
        self.m[0][0] * self.m[1][2] * self.m[2][1] * self.m[3][3] -
        self.m[0][1] * self.m[1][0] * self.m[3][2] * self.m[3][3] +
        self.m[0][0] * self.m[1][1] * self.m[3][2] * self.m[3][3]
    }

    /// Multiplies all of the transform's component by a scalar and returns the result.
    #[must_use]
    #[inline]
    pub fn multiply_scalar(&self, x: f32) -> Self {
        Self::new(
            self.m[0][0] * x, self.m[0][1] * x, self.m[0][2] * x, self.m[0][3] * x,
            self.m[1][0] * x, self.m[1][1] * x, self.m[1][2] * x, self.m[1][3] * x,
            self.m[2][0] * x, self.m[2][1] * x, self.m[3][2] * x, self.m[2][3] * x,
            self.m[3][0] * x, self.m[3][1] * x, self.m[3][2] * x, self.m[3][3] * x
        )
    }

    /*

    #[inline]
    #[must_use]
    pub unsafe fn then_sse(&self, x: f32) -> Self { }
    #[inline]
    #[must_use]
    pub unsafe fn then_avx4(&self, x: f32) -> Self { }
    #[inline]
    #[must_use]
    pub unsafe fn then_avx8(&self, x: f32) -> Self { }

    #[inline]
    #[must_use]
    pub unsafe fn inverse_sse(&self, x: f32) -> Self { }
    #[inline]
    #[must_use]
    pub unsafe fn inverse_avx4(&self, x: f32) -> Self { }
    #[inline]
    #[must_use]
    pub unsafe fn inverse_avx8(&self, x: f32) -> Self { }

    #[inline]
    #[must_use]
    pub unsafe fn determinant_sse(&self) -> f32 { }
    #[inline]
    #[must_use]
    pub unsafe fn determinant_avx4(&self) -> f32 { }
    #[inline]
    #[must_use]
    pub unsafe fn determinant_avx8(&self) -> f32 { }

    #[inline]
    #[must_use]
    pub unsafe fn multiply_scalar_sse(&self, x: f32) -> Self { }
    #[inline]
    #[must_use]
    pub unsafe fn multiply_scalar_avx4(&self, x: f32) -> Self { }
    #[inline]
    #[must_use]
    pub unsafe fn multiply_scalar_avx8(&self, x: f32) -> Self { }

    */

    #[inline]
    pub fn make_rotation(
        rotation_origin: (f32, f32),
        degrees: f32,
        axis_x: f32,
        axis_y: f32,
        axis_z: f32,
    ) -> Self {

        let (origin_x, origin_y) = rotation_origin;
        let pre_transform = Self::new_translation(-origin_x, -origin_y, -0.0);
        let post_transform = Self::new_translation(origin_x, origin_y, 0.0);
        let theta = 2.0_f32 * core::f32::consts::PI - degrees.to_radians();
        let rotate_transform = Self::IDENTITY.then(&Self::new_rotation(axis_x, axis_y, axis_z, theta));

        pre_transform
        .then(&rotate_transform)
        .then(&post_transform)
    }
}
