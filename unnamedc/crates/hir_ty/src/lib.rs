use la_arena::{Arena, ArenaMap};

pub fn infer(program: &hir::Program) -> InferResult {
    infer_with_var_tys(program, ArenaMap::default())
}

pub fn infer_with_var_tys(
    program: &hir::Program,
    var_tys: ArenaMap<hir::VarDefIdx, Ty>,
) -> InferResult {
    let mut infer_ctx = InferCtx {
        result: InferResult { expr_tys: ArenaMap::default(), var_tys, errors: Vec::new() },
        var_defs: &program.var_defs,
        exprs: &program.exprs,
    };

    for stmt in &program.stmts {
        infer_ctx.infer_stmt(stmt);
    }

    infer_ctx.result
}

#[derive(Debug)]
pub struct InferResult {
    pub expr_tys: ArenaMap<hir::ExprIdx, Ty>,
    pub var_tys: ArenaMap<hir::VarDefIdx, Ty>,
    pub errors: Vec<TyError>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ty {
    Unknown,
    Int,
    String,
    Unit,
}

#[derive(Debug, PartialEq)]
pub struct TyError {
    pub expr: hir::ExprIdx,
    pub kind: TyErrorKind,
}

#[derive(Debug, PartialEq)]
pub enum TyErrorKind {
    Mismatch { expected: Ty, found: Ty },
}

struct InferCtx<'a> {
    result: InferResult,
    var_defs: &'a Arena<hir::VarDef>,
    exprs: &'a Arena<hir::Expr>,
}

impl InferCtx<'_> {
    fn infer_stmt(&mut self, stmt: &hir::Stmt) -> Ty {
        match stmt {
            hir::Stmt::VarDef(var_def) => {
                let value_ty = self.infer_expr(self.var_defs[*var_def].value);
                self.result.var_tys.insert(*var_def, value_ty);

                Ty::Unit
            }
            hir::Stmt::Expr(expr) => self.infer_expr(*expr),
        }
    }

    fn infer_expr(&mut self, expr: hir::ExprIdx) -> Ty {
        let ty = match self.exprs[expr] {
            hir::Expr::Missing => Ty::Unknown,

            hir::Expr::Bin { lhs, rhs, .. } => {
                let lhs_ty = self.infer_expr(lhs);
                let rhs_ty = self.infer_expr(rhs);

                let is_lhs_missing = self.exprs[lhs] == hir::Expr::Missing;
                let is_rhs_missing = self.exprs[rhs] == hir::Expr::Missing;
                let is_anything_missing = is_lhs_missing || is_rhs_missing;
                if !is_anything_missing {
                    for (expr, ty) in [(lhs, lhs_ty), (rhs, rhs_ty)] {
                        if ty != Ty::Int {
                            self.result.errors.push(TyError {
                                expr,
                                kind: TyErrorKind::Mismatch { expected: Ty::Int, found: ty },
                            });
                        }
                    }
                }

                Ty::Int
            }

            hir::Expr::Block(ref stmts) => match stmts.split_last() {
                Some((last, rest)) => {
                    for stmt in rest {
                        self.infer_stmt(stmt);
                    }

                    self.infer_stmt(last)
                }

                None => Ty::Unit,
            },

            hir::Expr::VarRef(var_def) => self.result.var_tys[var_def],

            hir::Expr::IntLiteral(_) => Ty::Int,

            hir::Expr::StringLiteral(_) => Ty::String,
        };

        self.result.expr_tys.insert(expr, ty);

        ty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_int_literal() {
        let mut exprs = Arena::new();
        let ten = exprs.alloc(hir::Expr::IntLiteral(10));

        let result = infer(&hir::Program {
            var_defs: Arena::new(),
            exprs,
            stmts: vec![hir::Stmt::Expr(ten)],
        });

        assert_eq!(result.expr_tys[ten], Ty::Int);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_string_literal() {
        let mut exprs = Arena::new();
        let hello = exprs.alloc(hir::Expr::StringLiteral("hello".to_string()));

        let result = infer(&hir::Program {
            var_defs: Arena::new(),
            exprs,
            stmts: vec![hir::Stmt::Expr(hello)],
        });

        assert_eq!(result.expr_tys[hello], Ty::String);
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
            var_defs: Arena::new(),
            exprs,
            stmts: vec![hir::Stmt::Expr(ten_times_twenty)],
        });

        assert_eq!(result.expr_tys[ten], Ty::Int);
        assert_eq!(result.expr_tys[twenty], Ty::Int);
        assert_eq!(result.expr_tys[ten_times_twenty], Ty::Int);
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
            var_defs: Arena::new(),
            exprs,
            stmts: vec![hir::Stmt::Expr(bin_expr)],
        });

        assert_eq!(result.expr_tys[string], Ty::String);
        assert_eq!(result.expr_tys[int], Ty::Int);
        assert_eq!(result.expr_tys[bin_expr], Ty::Int);
        assert_eq!(
            result.errors,
            [TyError {
                expr: string,
                kind: TyErrorKind::Mismatch { expected: Ty::Int, found: Ty::String }
            }]
        );
    }

    #[test]
    fn infer_var_def() {
        let mut var_defs = Arena::new();
        let mut exprs = Arena::new();

        let two = exprs.alloc(hir::Expr::IntLiteral(2));
        let var_def = var_defs.alloc(hir::VarDef { value: two });

        let result =
            infer(&hir::Program { var_defs, exprs, stmts: vec![hir::Stmt::VarDef(var_def)] });

        assert_eq!(result.expr_tys[two], Ty::Int);
        assert_eq!(result.var_tys[var_def], Ty::Int);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_chain_of_var_refs_and_defs() {
        let mut var_defs = Arena::new();
        let mut exprs = Arena::new();

        let string = exprs.alloc(hir::Expr::StringLiteral("test".to_string()));
        let a_def = var_defs.alloc(hir::VarDef { value: string });
        let a = exprs.alloc(hir::Expr::VarRef(a_def));
        let b_def = var_defs.alloc(hir::VarDef { value: a });
        let b = exprs.alloc(hir::Expr::VarRef(b_def));
        let c_def = var_defs.alloc(hir::VarDef { value: b });
        let c = exprs.alloc(hir::Expr::VarRef(c_def));

        let result = infer(&hir::Program {
            var_defs,
            exprs,
            stmts: vec![
                hir::Stmt::VarDef(a_def),
                hir::Stmt::VarDef(b_def),
                hir::Stmt::VarDef(c_def),
                hir::Stmt::Expr(c),
            ],
        });

        assert_eq!(result.expr_tys[string], Ty::String);
        assert_eq!(result.var_tys[a_def], Ty::String);
        assert_eq!(result.expr_tys[a], Ty::String);
        assert_eq!(result.var_tys[b_def], Ty::String);
        assert_eq!(result.expr_tys[b], Ty::String);
        assert_eq!(result.var_tys[c_def], Ty::String);
        assert_eq!(result.expr_tys[c], Ty::String);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_with_preserved_var_tys() {
        let (preserved_var_tys, var_defs, idx_def) = {
            let mut var_defs = Arena::new();
            let mut exprs = Arena::new();

            let six = exprs.alloc(hir::Expr::IntLiteral(6));
            let idx_def = var_defs.alloc(hir::VarDef { value: six });

            let result = infer(&hir::Program {
                var_defs: var_defs.clone(),
                exprs,
                stmts: vec![hir::Stmt::VarDef(idx_def)],
            });

            assert_eq!(result.expr_tys[six], Ty::Int);
            assert_eq!(result.var_tys[idx_def], Ty::Int);
            assert_eq!(result.errors, []);

            (result.var_tys, var_defs, idx_def)
        };

        let mut exprs = Arena::new();
        let idx = exprs.alloc(hir::Expr::VarRef(idx_def));

        let program = hir::Program { var_defs, exprs, stmts: vec![hir::Stmt::Expr(idx)] };
        let result = infer_with_var_tys(&program, preserved_var_tys);

        assert_eq!(result.expr_tys[idx], Ty::Int);
        assert_eq!(result.var_tys[idx_def], Ty::Int);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_missing_expr() {
        let mut exprs = Arena::new();
        let missing = exprs.alloc(hir::Expr::Missing);

        let result = infer(&hir::Program {
            var_defs: Arena::new(),
            exprs,
            stmts: vec![hir::Stmt::Expr(missing)],
        });

        assert_eq!(result.expr_tys[missing], Ty::Unknown);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn only_error_on_missing_expr_use() {
        let mut var_defs = Arena::new();
        let mut exprs = Arena::new();

        let missing = exprs.alloc(hir::Expr::Missing);
        let user_def = var_defs.alloc(hir::VarDef { value: missing });
        let user = exprs.alloc(hir::Expr::VarRef(user_def));
        let four = exprs.alloc(hir::Expr::IntLiteral(4));
        let user_plus_four =
            exprs.alloc(hir::Expr::Bin { lhs: user, rhs: four, op: Some(hir::BinOp::Add) });

        let result = infer(&hir::Program {
            var_defs,
            exprs,
            stmts: vec![hir::Stmt::VarDef(user_def), hir::Stmt::Expr(user_plus_four)],
        });

        assert_eq!(result.expr_tys[missing], Ty::Unknown);
        assert_eq!(result.expr_tys[user], Ty::Unknown);
        assert_eq!(result.expr_tys[four], Ty::Int);
        assert_eq!(result.expr_tys[user_plus_four], Ty::Int);
        assert_eq!(result.var_tys[user_def], Ty::Unknown);

        // we only get an error about `user`’s type not being known
        // until we try to do an operation with it
        assert_eq!(
            result.errors,
            [TyError {
                expr: user,
                kind: TyErrorKind::Mismatch { expected: Ty::Int, found: Ty::Unknown }
            }]
        );
    }

    #[test]
    fn dont_error_on_missing_expr_in_bin_expr() {
        let mut exprs = Arena::new();
        let ten = exprs.alloc(hir::Expr::IntLiteral(10));
        let missing = exprs.alloc(hir::Expr::Missing);
        let ten_times_missing =
            exprs.alloc(hir::Expr::Bin { lhs: ten, rhs: missing, op: Some(hir::BinOp::Mul) });

        let result = infer(&hir::Program {
            var_defs: Arena::new(),
            exprs,
            stmts: vec![hir::Stmt::Expr(ten_times_missing)],
        });

        assert_eq!(result.expr_tys[ten], Ty::Int);
        assert_eq!(result.expr_tys[missing], Ty::Unknown);
        assert_eq!(result.expr_tys[ten_times_missing], Ty::Int);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_empty_block() {
        let mut exprs = Arena::new();
        let block = exprs.alloc(hir::Expr::Block(Vec::new()));

        let result = infer(&hir::Program {
            var_defs: Arena::new(),
            exprs,
            stmts: vec![hir::Stmt::Expr(block)],
        });

        assert_eq!(result.expr_tys[block], Ty::Unit);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_block_ending_in_var_def() {
        let mut var_defs = Arena::new();
        let mut exprs = Arena::new();

        let string = exprs.alloc(hir::Expr::StringLiteral("🌈".to_string()));
        let var_def = var_defs.alloc(hir::VarDef { value: string });
        let block = exprs.alloc(hir::Expr::Block(vec![hir::Stmt::VarDef(var_def)]));

        let result = infer(&hir::Program { var_defs, exprs, stmts: vec![hir::Stmt::Expr(block)] });

        assert_eq!(result.expr_tys[string], Ty::String);
        assert_eq!(result.var_tys[var_def], Ty::String);
        assert_eq!(result.expr_tys[block], Ty::Unit);
        assert_eq!(result.errors, []);
    }

    #[test]
    fn infer_block_ending_in_expr() {
        let mut var_defs = Arena::new();
        let mut exprs = Arena::new();

        let seven = exprs.alloc(hir::Expr::IntLiteral(7));
        let num_def = var_defs.alloc(hir::VarDef { value: seven });
        let num = exprs.alloc(hir::Expr::VarRef(num_def));
        let block =
            exprs.alloc(hir::Expr::Block(vec![hir::Stmt::VarDef(num_def), hir::Stmt::Expr(num)]));

        let result = infer(&hir::Program { var_defs, exprs, stmts: vec![hir::Stmt::Expr(block)] });

        assert_eq!(result.expr_tys[seven], Ty::Int);
        assert_eq!(result.var_tys[num_def], Ty::Int);
        assert_eq!(result.expr_tys[num], Ty::Int);
        assert_eq!(result.expr_tys[block], Ty::Int);
        assert_eq!(result.errors, []);
    }
}
