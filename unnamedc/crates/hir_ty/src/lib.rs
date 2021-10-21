use arena::{Arena, ArenaMap};

pub fn infer(program: &hir::Program) -> InferResult {
    infer_in_scope(program, InScope::default())
}

pub fn infer_in_scope(program: &hir::Program, in_scope: InScope) -> InferResult {
    let mut infer_ctx = InferCtx {
        result: InferResult {
            local_tys: in_scope.local_tys,
            fnc_sigs: in_scope.fnc_sigs,
            param_tys: in_scope.param_tys,
            expr_tys: ArenaMap::default(),
            errors: Vec::new(),
        },
        local_defs: &program.local_defs,
        fnc_defs: &program.fnc_defs,
        params: &program.params,
        exprs: &program.exprs,
    };

    for stmt in &program.stmts {
        infer_ctx.infer_stmt(*stmt);
    }

    infer_ctx.result
}

#[derive(Debug)]
pub struct InferResult {
    local_tys: ArenaMap<hir::LocalDefIdx, hir::Ty>,
    fnc_sigs: ArenaMap<hir::FncDefIdx, Sig>,
    param_tys: ArenaMap<hir::ParamIdx, hir::Ty>,
    expr_tys: ArenaMap<hir::ExprIdx, hir::Ty>,
    errors: Vec<TyError>,
}

impl InferResult {
    pub fn in_scope(self) -> (InScope, Vec<TyError>) {
        let in_scope = InScope {
            local_tys: self.local_tys,
            fnc_sigs: self.fnc_sigs,
            param_tys: self.param_tys,
        };

        (in_scope, self.errors)
    }
}

#[derive(Debug, Clone, Default)]
pub struct InScope {
    local_tys: ArenaMap<hir::LocalDefIdx, hir::Ty>,
    fnc_sigs: ArenaMap<hir::FncDefIdx, Sig>,
    param_tys: ArenaMap<hir::ParamIdx, hir::Ty>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sig {
    params: Vec<hir::Ty>,
    ret_ty: hir::Ty,
}

#[derive(Debug, PartialEq)]
pub struct TyError {
    pub expr: hir::ExprIdx,
    pub kind: TyErrorKind,
}

#[derive(Debug, PartialEq)]
pub enum TyErrorKind {
    Mismatch { expected: hir::Ty, found: hir::Ty },
}

struct InferCtx<'a> {
    result: InferResult,
    local_defs: &'a Arena<hir::LocalDef>,
    fnc_defs: &'a Arena<hir::FncDef>,
    params: &'a Arena<hir::Param>,
    exprs: &'a Arena<hir::Expr>,
}

impl InferCtx<'_> {
    fn infer_stmt(&mut self, stmt: hir::Stmt) -> hir::Ty {
        match stmt {
            hir::Stmt::LocalDef(local_def) => {
                let value_ty = self.infer_expr(self.local_defs[local_def].value);
                self.result.local_tys.insert(local_def, value_ty);
            }
            hir::Stmt::FncDef(idx) => self.infer_fnc_def(idx),
            hir::Stmt::Expr(expr) => return self.infer_expr(expr),
        }

        hir::Ty::Unit
    }

    fn infer_fnc_def(&mut self, idx: arena::Idx<hir::FncDef>) {
        let fnc_def = self.fnc_defs[idx].clone();

        let mut params = Vec::with_capacity(fnc_def.params.len());

        for param_idx in fnc_def.params {
            let param = self.params[param_idx];
            params.push(param.ty);
            self.result.param_tys.insert(param_idx, param.ty);
        }

        let actual_ret_ty = self.infer_expr(fnc_def.body);
        self.expect_tys_match(fnc_def.body, fnc_def.ret_ty, actual_ret_ty);

        self.result.fnc_sigs.insert(idx, Sig { params, ret_ty: fnc_def.ret_ty });
    }

    fn infer_expr(&mut self, expr: hir::ExprIdx) -> hir::Ty {
        let ty = match self.exprs[expr] {
            hir::Expr::Missing => hir::Ty::Unknown,

            hir::Expr::Bin { lhs, rhs, .. } => {
                let lhs_ty = self.infer_expr(lhs);
                let rhs_ty = self.infer_expr(rhs);

                for (expr, ty) in [(lhs, lhs_ty), (rhs, rhs_ty)] {
                    self.expect_tys_match(expr, hir::Ty::S32, ty);
                }

                hir::Ty::S32
            }

            hir::Expr::Block(ref stmts) => match stmts.split_last() {
                Some((last, rest)) => {
                    for stmt in rest {
                        self.infer_stmt(*stmt);
                    }

                    self.infer_stmt(*last)
                }

                None => hir::Ty::Unit,
            },

            hir::Expr::VarRef(hir::VarDefIdx::Local(local_def)) => self.result.local_tys[local_def],

            hir::Expr::VarRef(hir::VarDefIdx::Param(param)) => self.result.param_tys[param],

            hir::Expr::IntLiteral(_) => hir::Ty::S32,

            hir::Expr::StringLiteral(_) => hir::Ty::String,
        };

        self.result.expr_tys.insert(expr, ty);

        ty
    }

    fn expect_tys_match(&mut self, expr: hir::ExprIdx, expected: hir::Ty, found: hir::Ty) {
        if found == expected || found == hir::Ty::Unknown || expected == hir::Ty::Unknown {
            return;
        }

        let expr = match &self.exprs[expr] {
            hir::Expr::Block(stmts) => stmts
                .last()
                .copied()
                .and_then(|stmt| match stmt {
                    hir::Stmt::Expr(e) => Some(e),
                    _ => None,
                })
                .unwrap_or(expr),
            _ => expr,
        };

        self.result.errors.push(TyError { expr, kind: TyErrorKind::Mismatch { expected, found } });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arena::IdxRange;

    #[test]
    fn infer_int_literal() {
        let mut exprs = Arena::new();
        let ten = exprs.alloc(hir::Expr::IntLiteral(10));

        let result =
            infer(&hir::Program { exprs, stmts: vec![hir::Stmt::Expr(ten)], ..Default::default() });

        assert_eq!(result.expr_tys[ten], hir::Ty::S32);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_string_literal() {
        let mut exprs = Arena::new();
        let hello = exprs.alloc(hir::Expr::StringLiteral("hello".to_string()));

        let result = infer(&hir::Program {
            exprs,
            stmts: vec![hir::Stmt::Expr(hello)],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[hello], hir::Ty::String);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_bin_expr_on_ints() {
        let mut exprs = Arena::new();
        let ten = exprs.alloc(hir::Expr::IntLiteral(10));
        let twenty = exprs.alloc(hir::Expr::IntLiteral(20));
        let ten_times_twenty =
            exprs.alloc(hir::Expr::Bin { lhs: ten, rhs: twenty, op: Some(hir::BinOp::Mul) });

        let result = infer(&hir::Program {
            exprs,
            stmts: vec![hir::Stmt::Expr(ten_times_twenty)],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[ten], hir::Ty::S32);
        assert_eq!(result.expr_tys[twenty], hir::Ty::S32);
        assert_eq!(result.expr_tys[ten_times_twenty], hir::Ty::S32);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_bin_expr_on_string_and_int() {
        let mut exprs = Arena::new();
        let string = exprs.alloc(hir::Expr::StringLiteral("100".to_string()));
        let int = exprs.alloc(hir::Expr::IntLiteral(7));
        let bin_expr =
            exprs.alloc(hir::Expr::Bin { lhs: string, rhs: int, op: Some(hir::BinOp::Sub) });

        let result = infer(&hir::Program {
            exprs,
            stmts: vec![hir::Stmt::Expr(bin_expr)],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[string], hir::Ty::String);
        assert_eq!(result.expr_tys[int], hir::Ty::S32);
        assert_eq!(result.expr_tys[bin_expr], hir::Ty::S32);
        assert_eq!(
            result.errors,
            [TyError {
                expr: string,
                kind: TyErrorKind::Mismatch { expected: hir::Ty::S32, found: hir::Ty::String }
            }]
        );
    }

    #[test]
    fn infer_local_def() {
        let mut local_defs = Arena::new();
        let mut exprs = Arena::new();

        let two = exprs.alloc(hir::Expr::IntLiteral(2));
        let local_def = local_defs.alloc(hir::LocalDef { value: two });

        let result = infer(&hir::Program {
            local_defs,
            exprs,
            stmts: vec![hir::Stmt::LocalDef(local_def)],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[two], hir::Ty::S32);
        assert_eq!(result.local_tys[local_def], hir::Ty::S32);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_chain_of_var_refs_and_defs() {
        let mut local_defs = Arena::new();
        let mut exprs = Arena::new();

        let string = exprs.alloc(hir::Expr::StringLiteral("test".to_string()));
        let a_def = local_defs.alloc(hir::LocalDef { value: string });
        let a = exprs.alloc(hir::Expr::VarRef(hir::VarDefIdx::Local(a_def)));
        let b_def = local_defs.alloc(hir::LocalDef { value: a });
        let b = exprs.alloc(hir::Expr::VarRef(hir::VarDefIdx::Local(b_def)));
        let c_def = local_defs.alloc(hir::LocalDef { value: b });
        let c = exprs.alloc(hir::Expr::VarRef(hir::VarDefIdx::Local(c_def)));

        let result = infer(&hir::Program {
            local_defs,
            exprs,
            stmts: vec![
                hir::Stmt::LocalDef(a_def),
                hir::Stmt::LocalDef(b_def),
                hir::Stmt::LocalDef(c_def),
                hir::Stmt::Expr(c),
            ],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[string], hir::Ty::String);
        assert_eq!(result.local_tys[a_def], hir::Ty::String);
        assert_eq!(result.expr_tys[a], hir::Ty::String);
        assert_eq!(result.local_tys[b_def], hir::Ty::String);
        assert_eq!(result.expr_tys[b], hir::Ty::String);
        assert_eq!(result.local_tys[c_def], hir::Ty::String);
        assert_eq!(result.expr_tys[c], hir::Ty::String);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_with_preserved_in_scope() {
        let (in_scope, local_defs, local_def) = {
            let mut local_defs = Arena::new();
            let mut exprs = Arena::new();

            let six = exprs.alloc(hir::Expr::IntLiteral(6));
            let local_def = local_defs.alloc(hir::LocalDef { value: six });

            let result = infer(&hir::Program {
                local_defs: local_defs.clone(),
                exprs,
                stmts: vec![hir::Stmt::LocalDef(local_def)],
                ..Default::default()
            });

            assert_eq!(result.expr_tys[six], hir::Ty::S32);
            assert_eq!(result.local_tys[local_def], hir::Ty::S32);
            assert_eq!(result.errors, []);

            (result.in_scope().0, local_defs, local_def)
        };

        let mut exprs = Arena::new();
        let local_value = exprs.alloc(hir::Expr::VarRef(hir::VarDefIdx::Local(local_def)));

        let program = hir::Program {
            local_defs,
            exprs,
            stmts: vec![hir::Stmt::Expr(local_value)],
            ..Default::default()
        };
        let result = infer_in_scope(&program, in_scope);

        assert_eq!(result.expr_tys[local_value], hir::Ty::S32);
        assert_eq!(result.local_tys[local_def], hir::Ty::S32);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_missing_expr() {
        let mut exprs = Arena::new();
        let missing = exprs.alloc(hir::Expr::Missing);

        let result = infer(&hir::Program {
            exprs,
            stmts: vec![hir::Stmt::Expr(missing)],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[missing], hir::Ty::Unknown);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn dont_error_on_missing_expr_use() {
        let mut local_defs = Arena::new();
        let mut exprs = Arena::new();

        let missing = exprs.alloc(hir::Expr::Missing);
        let user_def = local_defs.alloc(hir::LocalDef { value: missing });
        let user = exprs.alloc(hir::Expr::VarRef(hir::VarDefIdx::Local(user_def)));
        let four = exprs.alloc(hir::Expr::IntLiteral(4));
        let user_plus_four =
            exprs.alloc(hir::Expr::Bin { lhs: user, rhs: four, op: Some(hir::BinOp::Add) });

        let result = infer(&hir::Program {
            local_defs,
            exprs,
            stmts: vec![hir::Stmt::LocalDef(user_def), hir::Stmt::Expr(user_plus_four)],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[missing], hir::Ty::Unknown);
        assert_eq!(result.expr_tys[user], hir::Ty::Unknown);
        assert_eq!(result.expr_tys[four], hir::Ty::S32);
        assert_eq!(result.expr_tys[user_plus_four], hir::Ty::S32);
        assert_eq!(result.local_tys[user_def], hir::Ty::Unknown);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn dont_error_on_missing_expr_in_bin_expr() {
        let mut exprs = Arena::new();
        let ten = exprs.alloc(hir::Expr::IntLiteral(10));
        let missing = exprs.alloc(hir::Expr::Missing);
        let ten_times_missing =
            exprs.alloc(hir::Expr::Bin { lhs: ten, rhs: missing, op: Some(hir::BinOp::Mul) });

        let result = infer(&hir::Program {
            exprs,
            stmts: vec![hir::Stmt::Expr(ten_times_missing)],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[ten], hir::Ty::S32);
        assert_eq!(result.expr_tys[missing], hir::Ty::Unknown);
        assert_eq!(result.expr_tys[ten_times_missing], hir::Ty::S32);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_empty_block() {
        let mut exprs = Arena::new();
        let block = exprs.alloc(hir::Expr::Block(Vec::new()));

        let result = infer(&hir::Program {
            exprs,
            stmts: vec![hir::Stmt::Expr(block)],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[block], hir::Ty::Unit);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_block_ending_in_local_def() {
        let mut local_defs = Arena::new();
        let mut exprs = Arena::new();

        let string = exprs.alloc(hir::Expr::StringLiteral("🌈".to_string()));
        let local_def = local_defs.alloc(hir::LocalDef { value: string });
        let block = exprs.alloc(hir::Expr::Block(vec![hir::Stmt::LocalDef(local_def)]));

        let result = infer(&hir::Program {
            local_defs,
            exprs,
            stmts: vec![hir::Stmt::Expr(block)],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[string], hir::Ty::String);
        assert_eq!(result.local_tys[local_def], hir::Ty::String);
        assert_eq!(result.expr_tys[block], hir::Ty::Unit);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_block_ending_in_expr() {
        let mut local_defs = Arena::new();
        let mut exprs = Arena::new();

        let seven = exprs.alloc(hir::Expr::IntLiteral(7));
        let num_def = local_defs.alloc(hir::LocalDef { value: seven });
        let num = exprs.alloc(hir::Expr::VarRef(hir::VarDefIdx::Local(num_def)));
        let block =
            exprs.alloc(hir::Expr::Block(vec![hir::Stmt::LocalDef(num_def), hir::Stmt::Expr(num)]));

        let result = infer(&hir::Program {
            local_defs,
            exprs,
            stmts: vec![hir::Stmt::Expr(block)],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[seven], hir::Ty::S32);
        assert_eq!(result.local_tys[num_def], hir::Ty::S32);
        assert_eq!(result.expr_tys[num], hir::Ty::S32);
        assert_eq!(result.expr_tys[block], hir::Ty::S32);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_fnc_def_with_no_params() {
        let mut fnc_defs = Arena::new();
        let mut exprs = Arena::new();

        let empty_block = exprs.alloc(hir::Expr::Block(Vec::new()));
        let fnc_def = fnc_defs.alloc(hir::FncDef {
            params: IdxRange::default(),
            ret_ty: hir::Ty::Unit,
            body: empty_block,
        });

        let result = infer(&hir::Program {
            fnc_defs,
            exprs,
            stmts: vec![hir::Stmt::FncDef(fnc_def)],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[empty_block], hir::Ty::Unit);
        assert_eq!(result.fnc_sigs[fnc_def], Sig { params: Vec::new(), ret_ty: hir::Ty::Unit });
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_fnc_def_with_params() {
        let mut fnc_defs = Arena::new();
        let mut params = Arena::new();
        let mut exprs = Arena::new();

        let param_1 = params.alloc(hir::Param { ty: hir::Ty::S32 });
        let param_2 = params.alloc(hir::Param { ty: hir::Ty::S32 });
        let empty_block = exprs.alloc(hir::Expr::Block(Vec::new()));
        let fnc_def = fnc_defs.alloc(hir::FncDef {
            params: IdxRange::new_inclusive(param_1..=param_2),
            ret_ty: hir::Ty::Unit,
            body: empty_block,
        });

        let result = infer(&hir::Program {
            fnc_defs,
            params,
            exprs,
            stmts: vec![hir::Stmt::FncDef(fnc_def)],
            ..Default::default()
        });

        assert_eq!(result.param_tys[param_1], hir::Ty::S32);
        assert_eq!(result.param_tys[param_2], hir::Ty::S32);
        assert_eq!(result.expr_tys[empty_block], hir::Ty::Unit);
        assert_eq!(
            result.fnc_sigs[fnc_def],
            Sig { params: vec![hir::Ty::S32, hir::Ty::S32], ret_ty: hir::Ty::Unit }
        );
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_fnc_def_with_params_and_ret_ty() {
        let mut fnc_defs = Arena::new();
        let mut params = Arena::new();
        let mut exprs = Arena::new();

        let param_def = params.alloc(hir::Param { ty: hir::Ty::S32 });
        let param_ref = exprs.alloc(hir::Expr::VarRef(hir::VarDefIdx::Param(param_def)));
        let fnc_def = fnc_defs.alloc(hir::FncDef {
            params: IdxRange::new_inclusive(param_def..=param_def),
            ret_ty: hir::Ty::S32,
            body: param_ref,
        });

        let result = infer(&hir::Program {
            fnc_defs,
            params,
            exprs,
            stmts: vec![hir::Stmt::FncDef(fnc_def)],
            ..Default::default()
        });

        assert_eq!(result.param_tys[param_def], hir::Ty::S32);
        assert_eq!(result.expr_tys[param_ref], hir::Ty::S32);
        assert_eq!(
            result.fnc_sigs[fnc_def],
            Sig { params: vec![hir::Ty::S32], ret_ty: hir::Ty::S32 }
        );
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_fnc_def_with_mismatched_ret_ty() {
        let mut fnc_defs = Arena::new();
        let mut exprs = Arena::new();

        let string = exprs.alloc(hir::Expr::StringLiteral("hello".to_string()));
        let fnc_def = fnc_defs.alloc(hir::FncDef {
            params: IdxRange::default(),
            ret_ty: hir::Ty::Unit,
            body: string,
        });

        let result = infer(&hir::Program {
            fnc_defs,
            exprs,
            stmts: vec![hir::Stmt::FncDef(fnc_def)],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[string], hir::Ty::String);
        assert_eq!(result.fnc_sigs[fnc_def], Sig { params: Vec::new(), ret_ty: hir::Ty::Unit });
        assert_eq!(
            result.errors,
            [TyError {
                expr: string,
                kind: TyErrorKind::Mismatch { expected: hir::Ty::Unit, found: hir::Ty::String }
            }]
        );
    }

    #[test]
    fn avoid_mismatched_ret_ty_error_on_missing_fnc_body() {
        let mut fnc_defs = Arena::new();
        let mut exprs = Arena::new();

        let missing = exprs.alloc(hir::Expr::Missing);
        let fnc_def = fnc_defs.alloc(hir::FncDef {
            params: IdxRange::default(),
            ret_ty: hir::Ty::S32,
            body: missing,
        });

        let result = infer(&hir::Program {
            fnc_defs,
            exprs,
            stmts: vec![hir::Stmt::FncDef(fnc_def)],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[missing], hir::Ty::Unknown);
        assert_eq!(result.fnc_sigs[fnc_def], Sig { params: Vec::new(), ret_ty: hir::Ty::S32 });
        assert_eq!(result.errors, []);
    }

    #[test]
    fn avoid_mismatched_ret_ty_error_on_fnc_body_with_unknown_ty() {
        let mut fnc_defs = Arena::new();
        let mut exprs = Arena::new();

        let missing = exprs.alloc(hir::Expr::Missing);
        let fnc_def = fnc_defs.alloc(hir::FncDef {
            params: IdxRange::default(),
            ret_ty: hir::Ty::S32,
            body: missing,
        });

        let result = infer(&hir::Program {
            fnc_defs,
            exprs,
            stmts: vec![hir::Stmt::FncDef(fnc_def)],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[missing], hir::Ty::Unknown);
        assert_eq!(result.fnc_sigs[fnc_def], Sig { params: Vec::new(), ret_ty: hir::Ty::S32 });
        assert_eq!(result.errors, []);
    }

    #[test]
    fn avoid_mismatched_ret_ty_error_on_fnc_with_missing_ret_ty() {
        let mut fnc_defs = Arena::new();
        let mut exprs = Arena::new();

        let empty_block = exprs.alloc(hir::Expr::Block(Vec::new()));
        let fnc_def = fnc_defs.alloc(hir::FncDef {
            params: IdxRange::default(),
            ret_ty: hir::Ty::Unknown,
            body: empty_block,
        });

        let result = infer(&hir::Program {
            fnc_defs,
            exprs,
            stmts: vec![hir::Stmt::FncDef(fnc_def)],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[empty_block], hir::Ty::Unit);
        assert_eq!(result.fnc_sigs[fnc_def], Sig { params: Vec::new(), ret_ty: hir::Ty::Unknown });
        assert_eq!(result.errors, []);
    }

    #[test]
    fn show_mismatched_ty_error_on_last_expr_of_block() {
        let mut local_defs = Arena::new();
        let mut exprs = Arena::new();

        let string = exprs.alloc(hir::Expr::StringLiteral("foo".to_string()));
        let local_def = local_defs.alloc(hir::LocalDef { value: string });
        let local = exprs.alloc(hir::Expr::VarRef(hir::VarDefIdx::Local(local_def)));
        let block = exprs
            .alloc(hir::Expr::Block(vec![hir::Stmt::LocalDef(local_def), hir::Stmt::Expr(local)]));
        let ten = exprs.alloc(hir::Expr::IntLiteral(10));
        let block_plus_ten =
            exprs.alloc(hir::Expr::Bin { lhs: block, rhs: ten, op: Some(hir::BinOp::Add) });

        let result = infer(&hir::Program {
            local_defs,
            exprs,
            stmts: vec![hir::Stmt::Expr(block_plus_ten)],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[string], hir::Ty::String);
        assert_eq!(result.expr_tys[local], hir::Ty::String);
        assert_eq!(result.local_tys[local_def], hir::Ty::String);
        assert_eq!(result.expr_tys[block], hir::Ty::String);
        assert_eq!(result.expr_tys[ten], hir::Ty::S32);
        assert_eq!(result.expr_tys[block_plus_ten], hir::Ty::S32);
        assert_eq!(
            result.errors,
            [TyError {
                expr: local,
                kind: TyErrorKind::Mismatch { expected: hir::Ty::S32, found: hir::Ty::String }
            }]
        );
    }

    #[test]
    fn show_mismatched_ty_error_on_entire_block_if_last_stmt_is_not_expr() {
        let mut local_defs = Arena::new();
        let mut exprs = Arena::new();

        let five = exprs.alloc(hir::Expr::IntLiteral(5));
        let local_def = local_defs.alloc(hir::LocalDef { value: five });
        let block = exprs.alloc(hir::Expr::Block(vec![hir::Stmt::LocalDef(local_def)]));
        let four = exprs.alloc(hir::Expr::IntLiteral(4));
        let block_plus_four =
            exprs.alloc(hir::Expr::Bin { lhs: block, rhs: four, op: Some(hir::BinOp::Add) });

        let result = infer(&hir::Program {
            local_defs,
            exprs,
            stmts: vec![hir::Stmt::Expr(block_plus_four)],
            ..Default::default()
        });

        assert_eq!(result.expr_tys[five], hir::Ty::S32);
        assert_eq!(result.local_tys[local_def], hir::Ty::S32);
        assert_eq!(result.expr_tys[block], hir::Ty::Unit);
        assert_eq!(result.expr_tys[four], hir::Ty::S32);
        assert_eq!(result.expr_tys[block_plus_four], hir::Ty::S32);
        assert_eq!(
            result.errors,
            [TyError {
                expr: block,
                kind: TyErrorKind::Mismatch { expected: hir::Ty::S32, found: hir::Ty::Unit }
            }]
        );
    }
}
