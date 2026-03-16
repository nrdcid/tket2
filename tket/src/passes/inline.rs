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
#[derive(Clone, Debug, PartialEq, Eq, derive_more::Display, derive_more::Error)]
pub enum InlineError<N = Node> {
    /// Functions annotated with [InlineAnnotation::Always] form a cycle
    /// so inlining would produce an infinitely-big program
    #[display("Cycle detected in functions marked to Always inline: {_0}")]
    AlwaysCycle(Vec<N>),
}

/// A [ComposablePass] that inlines [Call]s to functions
/// according to [InlineAnnotation]s.
#[derive(Default, Clone, Debug)]
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

#[cfg(test)]
mod test {
    use rstest::rstest;
    use std::collections::HashSet;

    use hugr::{
        HugrView,
        builder::{
            Container, Dataflow, DataflowHugr, DataflowSubContainer, FunctionBuilder, HugrBuilder,
        },
        extension::prelude::{qb_t, usize_t},
        hugr::hugrmut::HugrMut,
        ops::handle::NodeHandle,
        types::Signature,
    };
    use hugr_passes::{ComposablePass, RemoveDeadFuncsPass, inline_dfgs::InlineDFGsPass};

    use super::{InlineAnnotation, InlineError, InlinePass};

    #[test]
    fn test_single_cycle() {
        let mut main = FunctionBuilder::new("main", Signature::new_endo([qb_t(), qb_t()])).unwrap();
        let mut mb = main.module_root_builder();
        let mut fb = mb
            .define_function("self-call", Signature::new_endo([qb_t()]))
            .unwrap();
        let c = fb
            .call::<true>(&fb.container_node().into(), &[], fb.input_wires())
            .unwrap();
        let fb = fb.finish_with_outputs(c.outputs()).unwrap();
        let inputs = main.input_wires();
        let mut hugr = main.finish_hugr_with_outputs(inputs).unwrap();
        hugr.set_metadata::<InlineAnnotation>(fb.node(), InlineAnnotation::Always);
        let backup = hugr.clone();

        // We error even though the function is not called
        let e = InlinePass::default().run(&mut hugr).unwrap_err();
        assert_eq!(e, InlineError::AlwaysCycle(vec![fb.node()]));
        assert_eq!(hugr, backup);

        RemoveDeadFuncsPass::default().run(&mut hugr).unwrap();
        assert_eq!(
            hugr.children(hugr.module_root()).collect::<Vec<_>>(),
            [hugr.entrypoint()]
        );
        let backup = hugr.clone();
        InlinePass::default().run(&mut hugr).unwrap();
        assert_eq!(hugr, backup);
    }

    #[test]
    fn cycle() {
        let mut main = FunctionBuilder::new("main", Signature::new_endo([usize_t()])).unwrap();
        let main_h = main.container_node().into();
        let mut mb = main.module_root_builder();
        let mut fb1 = mb
            .define_function("f1", Signature::new_endo([usize_t()]))
            .unwrap();
        let c1 = fb1.call::<true>(&main_h, &[], fb1.input_wires()).unwrap();
        let fb1 = fb1.finish_with_outputs(c1.outputs()).unwrap();
        let c2 = main.call(fb1.handle(), &[], main.input_wires()).unwrap();
        let mut hugr = main.finish_hugr_with_outputs(c2.outputs()).unwrap();
        hugr.set_metadata::<InlineAnnotation>(hugr.entrypoint(), InlineAnnotation::Always);
        InlinePass::default().run(&mut hugr.clone()).unwrap(); // Ok

        hugr.set_metadata::<InlineAnnotation>(fb1.node(), InlineAnnotation::Always);
        let e = InlinePass::default().run(&mut hugr).unwrap_err();
        assert_eq!(
            e,
            InlineError::AlwaysCycle(vec![fb1.node(), hugr.entrypoint()])
        );
    }

    #[rstest]
    fn test_one_deep(#[values(1, 2, 5)] num_calls: usize, #[values(false, true)] always: bool) {
        let mut main =
            FunctionBuilder::new("main", Signature::new_endo([qb_t(), qb_t(), qb_t()])).unwrap();

        let mut mb = main.module_root_builder();
        let swap = mb
            .define_function("called-once", Signature::new_endo([qb_t(), qb_t()]))
            .unwrap();
        let [a, b] = swap.input_wires_arr();
        let swap = swap.finish_with_outputs([b, a]).unwrap();

        let [mut a, mut b, c] = main.input_wires_arr();
        for _ in 0..num_calls {
            [a, b] = main.call(swap.handle(), &[], [a, b]).unwrap().outputs_arr();
        }
        let mut hugr = main.finish_hugr_with_outputs([a, b, c]).unwrap();
        if always {
            hugr.set_metadata::<InlineAnnotation>(swap.node(), InlineAnnotation::Always);
        }
        let backup = hugr.clone();

        InlinePass::default().run(&mut hugr).unwrap();
        hugr.validate().unwrap();
        let inlined = always || num_calls == 1;
        assert_eq!(
            hugr.static_targets(swap.node()).unwrap().count(),
            if inlined { 0 } else { num_calls }
        );
        if inlined {
            assert_eq!(hugr, backup);
        } else {
            InlineDFGsPass::default().run(&mut hugr).unwrap();
            hugr.validate().unwrap();
            let [inp, outp] = hugr.get_io(hugr.entrypoint()).unwrap();
            assert_eq!(
                HashSet::from_iter(hugr.input_neighbours(outp)),
                HashSet::from([inp])
            );
        }
    }
}
