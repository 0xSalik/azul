//! Event and callback filtering module
//!
//! # Example
//!
//! ```rust
//! let mut app_state = RefAny::new(AppState { counter: 5 });
//! let pipeline_id = PipelineId::new(0);
//!
//! // initial layout
//! let mut styled_dom = render_dom(&app_state, layout_info);
//! let mut layout_results = do_layout(&styled_dom);
//!
//! let mut previous_window_state = None;
//! let mut current_window_state = FulLWindowState::default();
//!
//! draw_display_list_to_screen(CachedDisplayList::new(&layout_results));
//!
//! loop { // window loop
//!
//!      // update the current_window_state from your preferred OS windowing library
//!      current_window_state.cursor = CursorPosition::InWindow(200, 500);
//!
//!      let events = Events::new(&current_window_state, &previous_window_state);
//!      let hit_test = HitTest::new(&current_window_state, &layout_results, &current_window_state.scroll_states);
//!
//!      previous_window_state = Some(current_window_state.clone());
//!      current_window_state.focused_node = hit_test.focused_node;
//!      current_window_state.hovered_nodes = hit_test.hovered_nodes;
//!
//!      let nodes_to_check = NodesToCheck::new(&hit_test, &events);
//!      let callbacks = CallbacksOfHitTest::new(&nodes_to_check, &events, &window.layout_results);
//!      let callback_result = call_callbacks(&callbacks, &hit_test);
//!
//!      if callbacks.update_screen = UpdateScreen::Relayout {
//!
//!         // redo the entire layout
//!         styled_dom = render_dom(&app_state, layout_info);
//!         layout_results = do_layout(&styled_dom);
//!         draw_display_list_to_screen(CachedDisplayList::new(&layout_results));
//!
//!      } else {
//!
//!           // only relayout what is necessary
//!           let style_and_layout_changes = StyleAndLayoutChanges::new(
//!               &nodes_to_check,
//!               &mut layout_results,
//!               &mut app_resources,
//!               &current_window_state.dimensions.size,
//!               pipeline_id,
//!               azul_layout::do_the_relayout
//!           );
//!
//!           if !style_and_layout_changes.is_empty() {
//!               draw_display_list_to_screen(CachedDisplayList::new(&layout_results));
//!           // } else if let Some(iframes) = style_and_layout_changes.get_iframes_to_relayout() { }
//!           // } else if let Some(gl_textures) = style_and_layout_changes.get_gltextures_to_redraw() { }
//!           } else {
//!               // nothing to do
//!           }
//!      }
//!
//!      #break; // - for doc test
//! }
//! ```rust

use std::collections::{HashSet, BTreeMap};
use crate::{
    FastHashMap,
    app_resources::AppResources,
    dom::{EventFilter, CallbackData, NotEventFilter, HoverEventFilter, FocusEventFilter, WindowEventFilter},
    callbacks:: {ScrollPosition, PipelineId, DomNodeId, HitTestItem, UpdateScreen},
    id_tree::NodeId,
    styled_dom::{DomId, ChangedCssProperty, AzNodeId},
    ui_solver::LayoutResult,
    window::{FullHitTest, RawWindowHandle, FullWindowState, ScrollStates, CallCallbacksResult},
};
use azul_css::{LayoutSize, CssProperty, LayoutPoint, LayoutRect};
#[cfg(feature = "opengl")]
use crate::gl::GlContextPtr;

#[derive(Debug, Clone, PartialEq)]
pub struct Events {
    pub window_events: Vec<WindowEventFilter>,
    pub hover_events: Vec<HoverEventFilter>,
    pub focus_events: Vec<FocusEventFilter>,
    pub old_hit_node_ids: BTreeMap<DomId, BTreeMap<NodeId, HitTestItem>>,
    pub old_focus_node: Option<DomNodeId>,
    pub current_window_state_mouse_is_down: bool,
    pub previous_window_state_mouse_is_down: bool,
    pub event_was_mouse_down: bool,
    pub event_was_mouse_leave: bool,
    pub event_was_mouse_release: bool,
}

impl Events {
    pub fn new(current_window_state: &FullWindowState, previous_window_state: &Option<FullWindowState>) -> Self {

        let mut current_window_events = get_window_events(current_window_state, previous_window_state);
        let mut current_hover_events = get_hover_events(&current_window_events);
        let mut current_focus_events = get_focus_events(&current_hover_events);

        let event_was_mouse_down    = current_window_events.contains(&WindowEventFilter::MouseDown);
        let event_was_mouse_release = current_window_events.contains(&WindowEventFilter::MouseUp);
        let event_was_mouse_leave   = current_window_events.contains(&WindowEventFilter::MouseLeave);
        let current_window_state_mouse_is_down = current_window_state.mouse_state.mouse_down();
        let previous_window_state_mouse_is_down = previous_window_state.as_ref().map(|f| f.mouse_state.mouse_down()).unwrap_or(false);

        let old_focus_node = previous_window_state.as_ref().and_then(|f| f.focused_node.clone());
        let old_hit_node_ids = previous_window_state.as_ref().map(|f| f.hovered_nodes.iter().map(|(dom_id, hit_test)| (*dom_id, hit_test.regular_hit_test_nodes.clone())).collect()).unwrap_or_default();

        if let Some(prev_state) = previous_window_state.as_ref() {
            if prev_state.theme != current_window_state.theme {
                current_window_events.insert(WindowEventFilter::ThemeChanged);
            }
            if current_window_state.hovered_nodes != prev_state.hovered_nodes.clone() {
                current_hover_events.insert(HoverEventFilter::MouseLeave);
                current_hover_events.insert(HoverEventFilter::MouseEnter);
            }
        }

        // even if there are no window events, the focus node can changed
        if current_window_state.focused_node != old_focus_node {
            current_focus_events.insert(FocusEventFilter::FocusReceived);
            current_focus_events.insert(FocusEventFilter::FocusLost);
        }

        let current_hover_events = current_hover_events.into_iter().collect::<Vec<_>>();
        let current_focus_events = current_focus_events.into_iter().collect::<Vec<_>>();
        let current_window_events = current_window_events.into_iter().collect::<Vec<_>>();

        Events {
            window_events: current_window_events,
            hover_events: current_hover_events,
            focus_events: current_focus_events,
            event_was_mouse_down,
            event_was_mouse_release,
            event_was_mouse_leave,
            current_window_state_mouse_is_down,
            previous_window_state_mouse_is_down,
            old_focus_node,
            old_hit_node_ids,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.window_events.is_empty() && self.hover_events.is_empty() && self.focus_events.is_empty()
    }

    pub fn event_was_mouse_scroll(&self) -> bool {
        // TODO: also need to look at TouchStart / TouchDrag
        self.window_events.contains(&WindowEventFilter::Scroll)
    }

    pub fn needs_hit_test(&self) -> bool {
        !(self.hover_events.is_empty() && self.focus_events.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodesToCheck {
    pub new_hit_node_ids: BTreeMap<DomId, BTreeMap<NodeId, HitTestItem>>,
    pub old_hit_node_ids: BTreeMap<DomId, BTreeMap<NodeId, HitTestItem>>,
    pub onmouseenter_nodes: BTreeMap<DomId, BTreeMap<NodeId, HitTestItem>>,
    pub onmouseleave_nodes: BTreeMap<DomId, BTreeMap<NodeId, HitTestItem>>,
    pub old_focus_node: Option<DomNodeId>,
    pub new_focus_node: Option<DomNodeId>,
    pub current_window_state_mouse_is_down: bool,
}

impl NodesToCheck {

    /// Determine which nodes are even relevant for callbacks or restyling
    pub fn new(hit_test: &FullHitTest, events: &Events) -> Self {
        // TODO: If the current mouse is down, but the event wasn't a click, that means it was a drag

        // Figure out what the hovered NodeIds are
        let new_hit_node_ids = if events.event_was_mouse_leave {
            BTreeMap::new()
        } else {
            hit_test.hovered_nodes.iter().map(|(k, v)| (k.clone(), v.regular_hit_test_nodes.clone())).collect()
        };

        // Figure out what the current focused NodeId is
        let new_focus_node = if events.event_was_mouse_down || events.event_was_mouse_release {
            hit_test.focused_node.clone().map(|o| DomNodeId { dom: o.0, node: AzNodeId::from_crate_internal(Some(o.1)) })
        } else {
            events.old_focus_node.clone()
        };

        // Collect all On::MouseEnter nodes (for both hover and focus events)
        let onmouseenter_nodes = new_hit_node_ids.iter().filter_map(|(dom_id, nhnid)| {
            let old_hit_node_ids = events.old_hit_node_ids.get(dom_id)?;
            let new = nhnid.iter()
            .filter(|(current_node_id, _)| old_hit_node_ids.get(current_node_id).is_none())
            .map(|(x, y)| (*x, y.clone()))
            .collect::<BTreeMap<_, _>>();
            if new.is_empty() { None } else { Some((*dom_id, new)) }
        }).collect::<BTreeMap<_, _>>();

        // Collect all On::MouseLeave nodes (for both hover and focus events)
        let onmouseleave_nodes = events.old_hit_node_ids.iter().filter_map(|(dom_id, ohnid)| {
            let old = ohnid
            .iter()
            .filter(|(prev_node_id, _)| new_hit_node_ids.get(dom_id).and_then(|d| d.get(prev_node_id)).is_none())
            .map(|(x, y)| (*x, y.clone()))
            .collect::<BTreeMap<_, _>>();
            if old.is_empty() { None } else { Some((*dom_id, old)) }
        }).collect::<BTreeMap<_, _>>();

        NodesToCheck {
            new_hit_node_ids: new_hit_node_ids,
            old_hit_node_ids: events.old_hit_node_ids.clone(),
            onmouseenter_nodes,
            onmouseleave_nodes,
            old_focus_node: events.old_focus_node.clone(),
            new_focus_node: new_focus_node,
            current_window_state_mouse_is_down: events.current_window_state_mouse_is_down,
        }
    }

    pub fn empty(mouse_down: bool) -> Self {
        Self {
            new_hit_node_ids: BTreeMap::new(),
            old_hit_node_ids: BTreeMap::new(),
            onmouseenter_nodes: BTreeMap::new(),
            onmouseleave_nodes: BTreeMap::new(),
            old_focus_node: None,
            new_focus_node: None,
            current_window_state_mouse_is_down: mouse_down,
        }
    }

    pub fn needs_hover_active_restyle(&self) -> bool {
        !(self.onmouseenter_nodes.is_empty() && self.onmouseleave_nodes.is_empty())
    }

    pub fn needs_focus_result(&self) -> bool {
        self.old_focus_node != self.new_focus_node
    }
}

pub type RestyleNodes = BTreeMap<NodeId, Vec<ChangedCssProperty>>;
pub type RelayoutNodes = BTreeMap<NodeId, Vec<ChangedCssProperty>>;

/// Style and layout changes
#[derive(Debug, Clone, PartialEq)]
pub struct StyleAndLayoutChanges {
    /// Changes that were made to style properties of nodes
    pub style_changes: BTreeMap<DomId, RestyleNodes>,
    /// Changes that were made to layout properties of nodes
    pub layout_changes: BTreeMap<DomId, RelayoutNodes>,
    /// Used to call `On::Resize` handlers
    pub nodes_that_changed_size: BTreeMap<DomId, Vec<NodeId>>,
}

impl StyleAndLayoutChanges {
    /// Determines and immediately applies the changes to the layout results
    pub fn new(
        nodes: &NodesToCheck,
        layout_results: &mut [LayoutResult],
        app_resources: &mut AppResources,
        window_size: LayoutSize,
        pipeline_id: PipelineId,
        css_changes: &BTreeMap<DomId, BTreeMap<NodeId, Vec<CssProperty>>>,
        callbacks_new_focus: &Option<Option<DomNodeId>>,
        relayout_cb: fn(LayoutRect, &mut LayoutResult, &mut AppResources, PipelineId, &RelayoutNodes) -> Vec<NodeId>
    ) -> StyleAndLayoutChanges {

        // immediately restyle the DOM to reflect the new :hover, :active and :focus nodes
        // and determine if the DOM needs a redraw or a relayout
        let mut style_changes = BTreeMap::new();
        let mut layout_changes = BTreeMap::new();
        let is_mouse_down = nodes.current_window_state_mouse_is_down;

        for (dom_id, onmouseenter_nodes) in nodes.onmouseenter_nodes.iter() {
            let layout_result = &mut layout_results[dom_id.inner];
            for onmouseenter_node_id in onmouseenter_nodes.keys() {
                // style :hover nodes

                let hover_node = &mut layout_result.styled_dom.styled_nodes.as_container_mut()[*onmouseenter_node_id];
                if hover_node.needs_hover_restyle() {
                    let style_props_changed = hover_node.restyle_hover();
                    let mut style_style_props = style_props_changed.iter().filter(|prop| !prop.previous_prop.get_type().can_trigger_relayout()).cloned().collect::<Vec<ChangedCssProperty>>();
                    let mut style_layout_props = style_props_changed.iter().filter(|prop| prop.previous_prop.get_type().can_trigger_relayout()).cloned().collect::<Vec<ChangedCssProperty>>();

                    if !style_style_props.is_empty() {
                        style_changes.entry(*dom_id).or_insert_with(|| BTreeMap::new()).entry(*onmouseenter_node_id).or_insert_with(|| Vec::new()).append(&mut style_style_props);
                    }
                    if !style_layout_props.is_empty() {
                        layout_changes.entry(*dom_id).or_insert_with(|| BTreeMap::new()).entry(*onmouseenter_node_id).or_insert_with(|| Vec::new()).append(&mut style_layout_props);
                    }
                }

                if is_mouse_down {
                    // style :active nodes
                    if hover_node.needs_active_restyle() {
                        let style_props_changed = hover_node.restyle_active();
                        let mut style_style_props = style_props_changed.iter().filter(|prop| !prop.previous_prop.get_type().can_trigger_relayout()).cloned().collect::<Vec<ChangedCssProperty>>();
                        let mut style_layout_props = style_props_changed.iter().filter(|prop| prop.previous_prop.get_type().can_trigger_relayout()).cloned().collect::<Vec<ChangedCssProperty>>();

                        if !style_style_props.is_empty() {
                            style_changes.entry(*dom_id).or_insert_with(|| BTreeMap::new()).entry(*onmouseenter_node_id).or_insert_with(|| Vec::new()).append(&mut style_style_props);
                        }
                        if !style_layout_props.is_empty() {
                            layout_changes.entry(*dom_id).or_insert_with(|| BTreeMap::new()).entry(*onmouseenter_node_id).or_insert_with(|| Vec::new()).append(&mut style_layout_props);
                        }
                    }
                }
            }
        }

        for (dom_id, onmouseleave_nodes) in nodes.onmouseleave_nodes.iter() {
            let layout_result = &mut layout_results[dom_id.inner];
            for onmouseleave_node_id in onmouseleave_nodes.keys() {
                // style :hover nodes

                let hover_node = &mut layout_result.styled_dom.styled_nodes.as_container_mut()[*onmouseleave_node_id];
                if hover_node.needs_hover_restyle() {
                    let style_props_changed = hover_node.restyle_hover();
                    let mut style_style_props = style_props_changed.iter().filter(|prop| !prop.previous_prop.get_type().can_trigger_relayout()).cloned().collect::<Vec<ChangedCssProperty>>();
                    let mut style_layout_props = style_props_changed.iter().filter(|prop| prop.previous_prop.get_type().can_trigger_relayout()).cloned().collect::<Vec<ChangedCssProperty>>();

                    if !style_style_props.is_empty() {
                        style_changes.entry(*dom_id).or_insert_with(|| BTreeMap::new()).entry(*onmouseleave_node_id).or_insert_with(|| Vec::new()).append(&mut style_style_props);
                    }
                    if !style_layout_props.is_empty() {
                        layout_changes.entry(*dom_id).or_insert_with(|| BTreeMap::new()).entry(*onmouseleave_node_id).or_insert_with(|| Vec::new()).append(&mut style_layout_props);
                    }
                }

                if is_mouse_down {
                    // style :active nodes
                    if hover_node.needs_active_restyle() {
                        let style_props_changed = hover_node.restyle_active();
                        let mut style_style_props = style_props_changed.iter().filter(|prop| !prop.previous_prop.get_type().can_trigger_relayout()).cloned().collect::<Vec<ChangedCssProperty>>();
                        let mut style_layout_props = style_props_changed.iter().filter(|prop| prop.previous_prop.get_type().can_trigger_relayout()).cloned().collect::<Vec<ChangedCssProperty>>();

                        if !style_style_props.is_empty() {
                            style_changes.entry(*dom_id).or_insert_with(|| BTreeMap::new()).entry(*onmouseleave_node_id).or_insert_with(|| Vec::new()).append(&mut style_style_props);
                        }
                        if !style_layout_props.is_empty() {
                            layout_changes.entry(*dom_id).or_insert_with(|| BTreeMap::new()).entry(*onmouseleave_node_id).or_insert_with(|| Vec::new()).append(&mut style_layout_props);
                        }
                    }
                }
            }
        }

        let new_focus_node = if let Some(new) = callbacks_new_focus.as_ref() { new } else { &nodes.new_focus_node };

        if nodes.old_focus_node != *new_focus_node {

            if let Some(DomNodeId { dom, node }) = nodes.old_focus_node {
                let layout_result = &mut layout_results[dom.inner];
                let node = node.into_crate_internal().unwrap();
                let old_focus_node = &mut layout_result.styled_dom.styled_nodes.as_container_mut()[node];
                if old_focus_node.needs_focus_restyle() {
                    let style_props_changed = old_focus_node.restyle_focus();
                    let mut style_style_props = style_props_changed.iter().filter(|prop| !prop.previous_prop.get_type().can_trigger_relayout()).cloned().collect::<Vec<ChangedCssProperty>>();
                    let mut style_layout_props = style_props_changed.iter().filter(|prop| prop.previous_prop.get_type().can_trigger_relayout()).cloned().collect::<Vec<ChangedCssProperty>>();

                    if !style_style_props.is_empty() {
                        style_changes.entry(dom).or_insert_with(|| BTreeMap::new()).entry(node).or_insert_with(|| Vec::new()).append(&mut style_style_props);
                    }
                    if !style_layout_props.is_empty() {
                        layout_changes.entry(dom).or_insert_with(|| BTreeMap::new()).entry(node).or_insert_with(|| Vec::new()).append(&mut style_layout_props);
                    }
                }
            }

            if let Some(DomNodeId { dom, node }) = *new_focus_node {
                let layout_result = &mut layout_results[dom.inner];
                let node = node.into_crate_internal().unwrap();
                let new_focus_node = &mut layout_result.styled_dom.styled_nodes.as_container_mut()[node];
                if new_focus_node.needs_focus_restyle() {
                    let style_props_changed = new_focus_node.restyle_focus();
                    let mut style_style_props = style_props_changed.iter().filter(|prop| !prop.previous_prop.get_type().can_trigger_relayout()).cloned().collect::<Vec<ChangedCssProperty>>();
                    let mut style_layout_props = style_props_changed.iter().filter(|prop| prop.previous_prop.get_type().can_trigger_relayout()).cloned().collect::<Vec<ChangedCssProperty>>();

                    if !style_style_props.is_empty() {
                        style_changes.entry(dom).or_insert_with(|| BTreeMap::new()).entry(node).or_insert_with(|| Vec::new()).append(&mut style_style_props);
                    }
                    if !style_layout_props.is_empty() {
                        layout_changes.entry(dom).or_insert_with(|| BTreeMap::new()).entry(node).or_insert_with(|| Vec::new()).append(&mut style_layout_props);
                    }
                }
            }
        }

        // restyle all the nodes according to the existing_changed_styles
        for (dom_id, existing_changes_map) in css_changes.iter() {
            for (node_id, changed_css_property_vec) in existing_changes_map.iter() {
                for changed_css_property in changed_css_property_vec.iter() {
                    if let Some(changed_prop) = layout_results[dom_id.inner].styled_dom.styled_nodes.as_container_mut()[*node_id].restyle_single_property(changed_css_property) {
                        // css property changed, now figure out if it was a style or a layout prop
                        if changed_prop.previous_prop.get_type().can_trigger_relayout() {
                            layout_changes.entry(*dom_id).or_insert_with(|| BTreeMap::default()).entry(*node_id).or_insert_with(|| Vec::new()).push(changed_prop);
                        } else {
                            style_changes.entry(*dom_id).or_insert_with(|| BTreeMap::default()).entry(*node_id).or_insert_with(|| Vec::new()).push(changed_prop);
                        }
                    }
                }
            }
        }

        let nodes_that_changed_size = layout_changes.iter().filter_map(|(dom_id, relayout_nodes)| {
            if relayout_nodes.is_empty() { return None; }
            let parent_rect = match layout_results[dom_id.inner].parent_dom_id.as_ref() {
                None => LayoutRect::new(LayoutPoint::zero(), window_size),
                Some(parent_dom_id) => {
                    let parent_layout_result = &layout_results[parent_dom_id.inner];
                    let parent_iframe_node_id = parent_layout_result.iframe_mapping.iter().find_map(|(k, v)| if *v == *dom_id { Some(*k) } else { None }).unwrap();
                    parent_layout_result.rects.as_ref()[parent_iframe_node_id].get_approximate_static_bounds()
                }
            };
            let nodes_that_changed_size = (relayout_cb)(parent_rect, &mut layout_results[dom_id.inner], app_resources, pipeline_id, relayout_nodes);
            if !nodes_that_changed_size.is_empty() { Some((*dom_id, nodes_that_changed_size)) } else { None }
        }).collect();

        StyleAndLayoutChanges {
            style_changes,
            layout_changes,
            nodes_that_changed_size,
        }
    }

    // Note: this can be false in case that only opacity: / transform: properties changed!
    pub fn need_regenerate_display_list(&self) -> bool {
        if !self.need_redraw() { return false; }
        // is_gpu_only_property = is the changed CSS property an opacity / transform / rotate property (which doesn't require to regenerate the display list)
        self.style_changes.iter().all(|(_, restyle_nodes)| {
            restyle_nodes.iter().all(|(_, changed_css_properties)| {
                changed_css_properties.iter().all(|changed_prop| changed_prop.current_prop.get_type().is_gpu_only_property())
            })
        })
    }

    pub fn need_redraw(&self) -> bool {
        !self.style_changes.is_empty() && !self.layout_changes.is_empty() && !self.nodes_that_changed_size.is_empty()
    }
}


#[derive(Debug, Clone, PartialEq)]
pub struct CallbackToCall {
    pub node_id: NodeId,
    pub hit_test_item: Option<HitTestItem>,
    pub callback: CallbackData,
}

#[derive(Debug, Clone)]
pub struct CallbacksOfHitTest {
    /// A BTreeMap where each item is already filtered by the proper hit-testing type,
    /// meaning in order to get the proper callbacks, you simply have to iterate through
    /// all node IDs
    pub nodes_with_callbacks: BTreeMap<DomId, Vec<CallbackToCall>>,
}

impl CallbacksOfHitTest {

    /// Determine which event / which callback(s) should be called and in which order
    ///
    /// This function also updates / mutates the current window states `focused_node`
    /// as well as the `window_state.previous_state`
    pub fn new(nodes_to_check: &NodesToCheck, events: &Events, layout_results: &[LayoutResult]) -> Self {

        use crate::dom::{ComponentEventFilter, ApplicationEventFilter};

        let mut nodes_with_callbacks = BTreeMap::new();

        for (dom_id, layout_result) in layout_results.iter().enumerate() {
            let dom_id = DomId { inner: dom_id };
            // iterate through all callbacks of all nodes
            for (node_id, node_data) in layout_result.styled_dom.node_data.as_ref().iter().enumerate() {
                let node_id = NodeId::new(node_id);
                let az_node_id = AzNodeId::from_crate_internal(Some(node_id));
                for callback in node_data.get_callbacks().iter() {
                    // see if the callback matches
                    match callback.event {
                        EventFilter::Window(wev) => {
                            if events.window_events.contains(&wev) {
                                nodes_with_callbacks.entry(dom_id).or_insert_with(|| Vec::new()).push(CallbackToCall {
                                    callback: callback.clone(),
                                    hit_test_item: None,
                                    node_id,
                                })
                            }
                        },
                        EventFilter::Hover(HoverEventFilter::MouseEnter) => {
                            if let Some(hit_test_item) = nodes_to_check.onmouseenter_nodes.get(&dom_id).and_then(|n| n.get(&node_id)) {
                                nodes_with_callbacks.entry(dom_id).or_insert_with(|| Vec::new()).push(CallbackToCall {
                                    callback: callback.clone(),
                                    hit_test_item: Some(*hit_test_item),
                                    node_id,
                                });
                            }
                        },
                        EventFilter::Hover(HoverEventFilter::MouseLeave) => {
                            if let Some(hit_test_item) = nodes_to_check.onmouseleave_nodes.get(&dom_id).and_then(|n| n.get(&node_id)) {
                                nodes_with_callbacks.entry(dom_id).or_insert_with(|| Vec::new()).push(CallbackToCall {
                                    callback: callback.clone(),
                                    hit_test_item: Some(*hit_test_item),
                                    node_id,
                                });
                            }
                        },
                        EventFilter::Hover(hev) => {
                            if let Some(hit_test_item) = nodes_to_check.new_hit_node_ids.get(&dom_id).and_then(|n| n.get(&node_id)) {
                                if events.hover_events.contains(&hev) {
                                    nodes_with_callbacks.entry(dom_id).or_insert_with(|| Vec::new()).push(CallbackToCall {
                                        callback: callback.clone(),
                                        hit_test_item: Some(*hit_test_item),
                                        node_id,
                                    });
                                }
                            }
                        },
                        EventFilter::Focus(FocusEventFilter::FocusReceived) => {
                            if nodes_to_check.new_focus_node == Some(DomNodeId { dom: dom_id, node: az_node_id }) && nodes_to_check.old_focus_node != nodes_to_check.new_focus_node {
                                nodes_with_callbacks.entry(dom_id).or_insert_with(|| Vec::new()).push(CallbackToCall {
                                    callback: callback.clone(),
                                    hit_test_item: None,
                                    node_id,
                                });
                            }
                        },
                        EventFilter::Focus(FocusEventFilter::FocusLost) => {
                            if nodes_to_check.old_focus_node == Some(DomNodeId { dom: layout_result.dom_id, node: az_node_id }) && nodes_to_check.old_focus_node != nodes_to_check.new_focus_node {
                                nodes_with_callbacks.entry(dom_id).or_insert_with(|| Vec::new()).push(CallbackToCall {
                                    callback: callback.clone(),
                                    hit_test_item: None,
                                    node_id,
                                });
                            }
                        },
                        EventFilter::Focus(fev) => {
                            if nodes_to_check.new_focus_node == Some(DomNodeId { dom: layout_result.dom_id, node: az_node_id }) && events.focus_events.contains(&fev) {
                                nodes_with_callbacks.entry(dom_id).or_insert_with(|| Vec::new()).push(CallbackToCall {
                                    callback: callback.clone(),
                                    hit_test_item: None,
                                    node_id,
                                });
                            }
                        },
                        EventFilter::Not(NotEventFilter::Focus(fev)) => {
                            if nodes_to_check.new_focus_node != Some(DomNodeId { dom: layout_result.dom_id, node: az_node_id }) && events.focus_events.contains(&fev) {
                                nodes_with_callbacks.entry(dom_id).or_insert_with(|| Vec::new()).push(CallbackToCall {
                                    callback: callback.clone(),
                                    hit_test_item: None,
                                    node_id,
                                });
                            }
                        },
                        EventFilter::Not(NotEventFilter::Hover(hev)) => {
                            if nodes_to_check.new_hit_node_ids.get(&dom_id).and_then(|n| n.get(&node_id)).is_none() && events.hover_events.contains(&hev) {
                                nodes_with_callbacks.entry(dom_id).or_insert_with(|| Vec::new()).push(CallbackToCall {
                                    callback: callback.clone(),
                                    hit_test_item: None,
                                    node_id,
                                });
                            }
                        },
                        EventFilter::Component(ComponentEventFilter::AfterMount) => { /* TODO - fire once for all newly created nodes! */ }
                        EventFilter::Component(ComponentEventFilter::BeforeUnmount) => { /* TODO - fire for all removed nodes! */ }
                        EventFilter::Component(ComponentEventFilter::NodeResized) => { /* TODO - fire for all resized nodes! */ }

                        EventFilter::Application(ApplicationEventFilter::DeviceConnected) => { /* TODO - fire if device connected! */ }
                        EventFilter::Application(ApplicationEventFilter::DeviceDisconnected) => { /* TODO - fire if device disconnected! */ }
                    }
                }
            }
        }

        CallbacksOfHitTest {
            nodes_with_callbacks,
        }
    }

    /// The actual function that calls the callbacks in their proper hierarchy and order
    #[cfg(feature = "opengl")]
    pub fn call(
        &mut self,
        full_window_state: &FullWindowState,
        raw_window_handle: &RawWindowHandle,
        scroll_states: &BTreeMap<DomId, BTreeMap<AzNodeId, ScrollPosition>>,
        gl_context: &GlContextPtr,
        layout_results: &mut [LayoutResult],
        modifiable_scroll_states: &mut ScrollStates,
        resources: &mut AppResources,
    ) -> CallCallbacksResult {

        use std::collections::BTreeSet;
        use crate::styled_dom::ParentWithNodeDepth;
        use crate::callbacks::CallbackInfo;
        use crate::window::LogicalPosition;

        let mut ret = CallCallbacksResult {
            should_scroll_render: false,
            callbacks_update_screen: UpdateScreen::DoNothing,
            modified_window_state: full_window_state.clone().into(),
            css_properties_changed: BTreeMap::new(),
            update_focused_node: None,
            timers: FastHashMap::new(),
            threads: FastHashMap::new(),
            windows_created: Vec::new(),
            cursor_changed: false,
        };
        let mut new_focus_target = None;
        let mut nodes_scrolled_in_callbacks = BTreeMap::<DomId, BTreeMap<AzNodeId, LogicalPosition>>::new();
        let current_cursor = full_window_state.mouse_state.mouse_cursor_type.clone();

        for (dom_id, callbacks_filter_list) in self.nodes_with_callbacks.iter_mut() {
            let layout_result = match layout_results.get(dom_id.inner) {
                Some(s) => s,
                None => { return ret; },
            };

            let mut callbacks = callbacks_filter_list
            .iter_mut()
            .map(|cbtc| (cbtc.node_id, (cbtc.hit_test_item, &mut cbtc.callback)))
            .collect::<BTreeMap<_, _>>();

            let mut blacklisted_event_types = BTreeSet::new();

            // Run all callbacks (front to back)
            for ParentWithNodeDepth { depth: _, node_id } in layout_result.styled_dom.non_leaf_nodes.as_ref().iter().rev() {
               let parent_node_id = node_id;
               for child_id in parent_node_id.into_crate_internal().unwrap().az_children(&layout_result.styled_dom.node_hierarchy.as_container()) {
                    if let Some((hit_test_item, callback_data)) = callbacks.get_mut(&child_id) {

                        if blacklisted_event_types.contains(&callback_data.event) {
                            continue;
                        }

                        let mut new_focus = None;
                        let mut stop_propagation = false;

                        let callback_info = CallbackInfo::new(
                            /*current_window_state:*/ &full_window_state,
                            /*modifiable_window_state:*/ &mut ret.modified_window_state,
                            /*gl_context,*/ gl_context,
                            /*resources,*/ resources,
                            /*timers:*/ &mut ret.timers,
                            /*threads:*/ &mut ret.threads,
                            /*new_windows:*/ &mut ret.windows_created,
                            /*current_window_handle:*/ raw_window_handle,
                            /*layout_results,*/ layout_results,
                            /*stop_propagation:*/ &mut stop_propagation,
                            /*focus_target:*/ &mut new_focus,
                            /*current_scroll_states:*/ scroll_states,
                            /*css_properties_changed_in_callbacks:*/ &mut ret.css_properties_changed,
                            /*nodes_scrolled_in_callback:*/ &mut nodes_scrolled_in_callbacks,
                            /*hit_dom_node:*/ DomNodeId { dom: *dom_id, node: AzNodeId::from_crate_internal(Some(child_id)) },
                            /*cursor_relative_to_item:*/ hit_test_item.as_ref().map(|hi| LayoutPoint::new(hi.point_relative_to_item.x, hi.point_relative_to_item.y)).into(),
                            /*cursor_in_viewport:*/ hit_test_item.as_ref().map(|hi| LayoutPoint::new(hi.point_in_viewport.x, hi.point_in_viewport.y)).into(),
                        );

                        // Invoke callback
                        let callback_return = (callback_data.callback.cb)(&mut callback_data.data, callback_info);

                        match callback_return {
                            UpdateScreen::RegenerateStyledDomForCurrentWindow => {
                                if ret.callbacks_update_screen == UpdateScreen::DoNothing { ret.callbacks_update_screen = callback_return;  }
                            },
                            UpdateScreen::RegenerateStyledDomForAllWindows => {
                                if ret.callbacks_update_screen == UpdateScreen::DoNothing || ret.callbacks_update_screen == UpdateScreen::RegenerateStyledDomForCurrentWindow  {
                                    ret.callbacks_update_screen = callback_return;
                                }
                            },
                            UpdateScreen::DoNothing => { }
                        }

                        if let Some(new_focus) = new_focus.clone() {
                            new_focus_target = Some(new_focus);
                        }

                        if stop_propagation {
                           blacklisted_event_types.insert(callback_data.event);
                        }
                    }
               }
            }

            // run the callbacks for node ID 0
            loop {
                if let Some((hit_test_item, callback_data)) = layout_result.styled_dom.root.into_crate_internal().and_then(|ci| callbacks.get_mut(&ci)) {

                    if blacklisted_event_types.contains(&callback_data.event) {
                        break; // break out of loop
                    }

                    let mut new_focus = None;
                    let mut stop_propagation = false;

                    let callback_info = CallbackInfo::new(
                        /*current_window_state:*/ &full_window_state,
                        /*modifiable_window_state:*/ &mut ret.modified_window_state,
                        /*gl_context,*/ gl_context,
                        /*resources,*/ resources,
                        /*timers:*/ &mut ret.timers,
                        /*threads:*/ &mut ret.threads,
                        /*new_windows:*/ &mut ret.windows_created,
                        /*current_window_handle:*/ raw_window_handle,
                        /*layout_results,*/ layout_results,
                        /*stop_propagation:*/ &mut stop_propagation,
                        /*focus_target:*/ &mut new_focus,
                        /*current_scroll_states:*/ scroll_states,
                        /*css_properties_changed_in_callbacks:*/ &mut ret.css_properties_changed,
                        /*nodes_scrolled_in_callback:*/ &mut nodes_scrolled_in_callbacks,
                        /*hit_dom_node:*/ DomNodeId { dom: *dom_id, node: layout_result.styled_dom.root },
                        /*cursor_relative_to_item:*/ hit_test_item.as_ref().map(|hi| LayoutPoint::new(hi.point_relative_to_item.x, hi.point_relative_to_item.y)).into(),
                        /*cursor_in_viewport:*/ hit_test_item.as_ref().map(|hi| LayoutPoint::new(hi.point_in_viewport.x, hi.point_in_viewport.y)).into(),
                    );

                    // Invoke callback
                    let callback_return = (callback_data.callback.cb)(&mut callback_data.data, callback_info);

                    match callback_return {
                        UpdateScreen::RegenerateStyledDomForCurrentWindow => {
                            if ret.callbacks_update_screen == UpdateScreen::DoNothing { ret.callbacks_update_screen = callback_return;  }
                        },
                        UpdateScreen::RegenerateStyledDomForAllWindows => {
                            if ret.callbacks_update_screen == UpdateScreen::DoNothing || ret.callbacks_update_screen == UpdateScreen::RegenerateStyledDomForCurrentWindow  {
                                ret.callbacks_update_screen = callback_return;
                            }
                        },
                        UpdateScreen::DoNothing => { }
                    }

                    if let Some(new_focus) = new_focus.clone() {
                        new_focus_target = Some(new_focus);
                    }

                    if stop_propagation {
                       blacklisted_event_types.insert(callback_data.event);
                    }
                }
                break;
            }
        }

        // Scroll nodes from programmatic callbacks
        for (dom_id, callback_scrolled_nodes) in nodes_scrolled_in_callbacks.iter() {
            let scrollable_nodes = &layout_results[dom_id.inner].scrollable_nodes;
            for (scroll_node_id, scroll_position) in callback_scrolled_nodes.iter() {
                let scroll_node = match scrollable_nodes.overflowing_nodes.get(&scroll_node_id) {
                    Some(s) => s,
                    None => continue,
                };

                modifiable_scroll_states.set_scroll_position(&scroll_node, *scroll_position);
                ret.should_scroll_render = true;
            }
        }

        let new_focus_node = new_focus_target.and_then(|ft| ft.resolve(&layout_results).ok()?);
        let focus_has_changed = full_window_state.focused_node != new_focus_node;

        if current_cursor != ret.modified_window_state.mouse_state.mouse_cursor_type {
            ret.cursor_changed = true;
        }

        if !focus_has_changed {
            ret.update_focused_node = None;
        } else {
            ret.update_focused_node = Some(new_focus_node);
        }

        ret
    }
}

fn get_window_events(current_window_state: &FullWindowState, previous_window_state: &Option<FullWindowState>) -> HashSet<WindowEventFilter> {

    use crate::window::CursorPosition::*;
    use crate::window::WindowPosition;

    let mut events_vec = HashSet::<WindowEventFilter>::new();

    let previous_window_state = match previous_window_state.as_ref() {
        Some(s) => s,
        None => return events_vec,
    };

    // resize, move, close events

    if current_window_state.flags.has_focus != previous_window_state.flags.has_focus {
        if current_window_state.flags.has_focus {
            events_vec.insert(WindowEventFilter::FocusReceived);
        } else {
            events_vec.insert(WindowEventFilter::FocusLost);
        }
    }

    if current_window_state.size.dimensions != previous_window_state.size.dimensions ||
       current_window_state.size.hidpi_factor != previous_window_state.size.hidpi_factor ||
       current_window_state.size.system_hidpi_factor != previous_window_state.size.system_hidpi_factor {
        events_vec.insert(WindowEventFilter::Resized);
    }

    match (current_window_state.position, previous_window_state.position) {
        (WindowPosition::Initialized(cur_pos), WindowPosition::Initialized(prev_pos)) => {
            if prev_pos != cur_pos {
                events_vec.insert(WindowEventFilter::Moved);
            }
        },
        (WindowPosition::Initialized(_), WindowPosition::Uninitialized) => {
            events_vec.insert(WindowEventFilter::Moved);
        },
        _ => { }
    }

    if current_window_state.flags.is_about_to_close {
        events_vec.insert(WindowEventFilter::CloseRequested);
    }

    // mouse move events

    match (previous_window_state.mouse_state.cursor_position, current_window_state.mouse_state.cursor_position) {
        (InWindow(_), OutOfWindow) |
        (InWindow(_), Uninitialized) => {
            events_vec.insert(WindowEventFilter::MouseLeave);
        },
        (OutOfWindow, InWindow(_)) |
        (Uninitialized, InWindow(_)) => {
            events_vec.insert(WindowEventFilter::MouseEnter);
        },
        (InWindow(a), InWindow(b)) => {
            if a != b {
                events_vec.insert(WindowEventFilter::MouseOver);
            }
        },
        _ => { },
    }

    if current_window_state.mouse_state.mouse_down() && !previous_window_state.mouse_state.mouse_down() {
        events_vec.insert(WindowEventFilter::MouseDown);
    }

    if current_window_state.mouse_state.left_down && !previous_window_state.mouse_state.left_down {
        events_vec.insert(WindowEventFilter::LeftMouseDown);
    }

    if current_window_state.mouse_state.right_down && !previous_window_state.mouse_state.right_down {
        events_vec.insert(WindowEventFilter::RightMouseDown);
    }

    if current_window_state.mouse_state.middle_down && !previous_window_state.mouse_state.middle_down {
        events_vec.insert(WindowEventFilter::MiddleMouseDown);
    }

    if previous_window_state.mouse_state.mouse_down() && !current_window_state.mouse_state.mouse_down() {
        events_vec.insert(WindowEventFilter::MouseUp);
    }

    if previous_window_state.mouse_state.left_down && !current_window_state.mouse_state.left_down {
        events_vec.insert(WindowEventFilter::LeftMouseUp);
    }

    if previous_window_state.mouse_state.right_down && !current_window_state.mouse_state.right_down {
        events_vec.insert(WindowEventFilter::RightMouseUp);
    }

    if previous_window_state.mouse_state.middle_down && !current_window_state.mouse_state.middle_down {
        events_vec.insert(WindowEventFilter::MiddleMouseUp);
    }

    // scroll events

    let is_scroll_previous =
        previous_window_state.mouse_state.scroll_x.is_some() ||
        previous_window_state.mouse_state.scroll_y.is_some();

    let is_scroll_now =
        current_window_state.mouse_state.scroll_x.is_some() ||
        current_window_state.mouse_state.scroll_y.is_some();

    if !is_scroll_previous && is_scroll_now {
        events_vec.insert(WindowEventFilter::ScrollStart);
    }

    if is_scroll_now {
        events_vec.insert(WindowEventFilter::Scroll);
    }

    if is_scroll_previous && !is_scroll_now {
        events_vec.insert(WindowEventFilter::ScrollEnd);
    }

    // keyboard events

    if previous_window_state.keyboard_state.current_virtual_keycode.is_none() && current_window_state.keyboard_state.current_virtual_keycode.is_some() {
        events_vec.insert(WindowEventFilter::VirtualKeyDown);
    }

    if current_window_state.keyboard_state.current_char.is_some() {
        events_vec.insert(WindowEventFilter::TextInput);
    }

    if previous_window_state.keyboard_state.current_virtual_keycode.is_some() && current_window_state.keyboard_state.current_virtual_keycode.is_none() {
        events_vec.insert(WindowEventFilter::VirtualKeyUp);
    }

    // misc events

    if previous_window_state.hovered_file.is_none() && current_window_state.hovered_file.is_some() {
        events_vec.insert(WindowEventFilter::HoveredFile);
    }

    if previous_window_state.hovered_file.is_some() && current_window_state.hovered_file.is_none() {
        if current_window_state.dropped_file.is_some() {
            events_vec.insert(WindowEventFilter::DroppedFile);
        } else {
            events_vec.insert(WindowEventFilter::HoveredFileCancelled);
        }
    }

    if current_window_state.theme != previous_window_state.theme {
        events_vec.insert(WindowEventFilter::ThemeChanged);
    }

    events_vec
}

fn get_hover_events(input: &HashSet<WindowEventFilter>) -> HashSet<HoverEventFilter> {
    input.iter().filter_map(|window_event| window_event.to_hover_event_filter()).collect()
}

fn get_focus_events(input: &HashSet<HoverEventFilter>) -> HashSet<FocusEventFilter> {
    input.iter().filter_map(|hover_event| hover_event.to_focus_event_filter()).collect()
}