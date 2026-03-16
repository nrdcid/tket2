use hugr::hugr::patch::inline_call::InlineCallError;
use hugr::hugr::patch::{Patch, inline_call::InlineCall};
use hugr_core::module_graph::{ModuleGraph, StaticNode};
use hugr_core::{Node, hugr::hugrmut::HugrMut, metadata::Metadata};
use hugr_passes::{ComposablePass, PassScope, composable::WithScope};

use itertools::Itertools;
use petgraph::algo::tarjan_scc;
use petgraph::data::DataMap;
use petgraph::visit::{IntoNodeReferences, NodeFiltered};
use serde::{Deserialize, Serialize};

/// Annotation that may be applied to functions to indicate
/// that/when calls to it should be inlined.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum InlineAnnotation {
    /// Always inline calls to this function.
    ///
    /// If this cannot be done, an error will be raised.
    Always,
    /// Leave inlining to the discretion of the optimizer.
    #[default]
    Auto,
}

impl Metadata for InlineAnnotation {
    type Type<'hugr> = InlineAnnotation;

    const KEY: &'static str = "tket.inline";
}

/// Errors that may be raised by [InlinePass]
#[derive(Clone, Debug, derive_more::Display, derive_more::Error)]
pub enum InlineError<N = Node> {
    /// Functions annotated with [InlineAnnotation::Always] form a cycle
    /// so inlining would produce an infinitely-big program
    #[display("Cycle detected in functions marked to Always inline: {_0}")]
    AlwaysCycle(Vec<N>),
}

/// A [ComposablePass] that inlines [Call]s to functions
/// according to [InlineAnnotation]s.
pub struct InlinePass {
    scope: PassScope,
}

impl WithScope for InlinePass {
    fn with_scope(self, scope: impl Into<PassScope>) -> Self {
        Self {
            scope: scope.into(),
        }
    }
}

impl<H: HugrMut> ComposablePass<H> for InlinePass {
    type Error = InlineError<H::Node>;
    type Result = ();

    fn run(&self, hugr: &mut H) -> Result<(), InlineError<H::Node>> {
        let cg = ModuleGraph::new(hugr);
        // TODO this is not correct for Entrypoint scopes, where we should only consider
        // reached functions.
        let always_funcs = NodeFiltered::from_fn(cg.graph(), |n| match cg.graph().node_weight(n) {
            Some(StaticNode::FuncDefn(n)) => {
                hugr.get_metadata::<InlineAnnotation>(*n)
                    .unwrap_or_default()
                    == InlineAnnotation::Always
            }
            _ => false,
        });
        let always_funcs_in_cycles = tarjan_scc(&always_funcs)
            .into_iter()
            .flat_map(|ns| {
                if let Ok(n) = ns.iter().exactly_one()
                    && cg.graph().edges_connecting(*n, *n).next().is_none()
                {
                    Vec::new() // Single-node SCC has no self edge, so discard
                } else {
                    ns.into_iter()
                        .map(|n| {
                            let StaticNode::FuncDefn(fd) = always_funcs.node_weight(n).unwrap()
                            else {
                                panic!("Expected only FuncDefns in sccs")
                            };
                            *fd
                        })
                        .collect()
                }
            })
            .collect::<Vec<_>>();
        if !always_funcs_in_cycles.is_empty() {
            return Err(InlineError::AlwaysCycle(always_funcs_in_cycles));
        }
        let always_funcs = always_funcs
            .node_references()
            .filter_map(|(_, sn)| match sn {
                StaticNode::FuncDefn(func) => Some(*func),
                _ => None,
            })
            .collect::<Vec<H::Node>>();
        for func in always_funcs {
            for (call, _) in hugr.static_targets(func).unwrap().collect::<Vec<_>>() {
                do_inline(call, hugr);
            }
            // Quick pass of dead-function elimination, to ease only-called-once inlining below
            if matches!(self.scope, PassScope::Global(_)) {
                assert_eq!(hugr.static_targets(func).unwrap().count(), 0);
                if !self.scope.preserve_interface(hugr).contains(&func) {
                    hugr.remove_subtree(func);
                }
            }
        }
        // Also inline any function called only once.
        let cg = ModuleGraph::new(hugr);
        let called_once = cg
            .graph()
            .node_references()
            .filter_map(|(_, sn)| match sn {
                StaticNode::FuncDefn(func) => hugr
                    .static_targets(*func)
                    .unwrap()
                    .collect_array::<1>()
                    .map(|[(call, _port)]| (*func, call)),

                _ => None,
            })
            .collect::<Vec<_>>();
        for (func, call) in called_once {
            do_inline(call, hugr);
            hugr.remove_subtree(func);
        }
        Ok(())
    }
}

fn do_inline<H: HugrMut>(call: H::Node, hugr: &mut H) {
    match InlineCall::new(call).apply(hugr) {
        Ok(()) => (),
        Err(InlineCallError::NotCallNode(_, _) | InlineCallError::CallTargetNotFuncDefn(_, _)) => {
            unreachable!();
        }
        Err(e) => {
            todo!("Update to handle {e:?}")
        }
    }
}
