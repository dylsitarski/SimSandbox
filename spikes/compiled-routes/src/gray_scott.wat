;; Route 4: Gray–Scott's rates written by hand in WebAssembly, as a block
;; written for a plugin interface would be. One pass over the cells of a
;; periodic n × n grid, x fastest; u, v, du, dv are byte addresses of
;; n·n doubles. The operations and their order are the typed program's:
;;   lap = ((west - 2c) + east)·inv + ((south - 2c) + north)·inv
;;   reaction = (r·u)·(v·v)
;;   du = ((Du·lap_u) - reaction) + F·(1 - u)
;;   dv = ((Dv·lap_v) + reaction) - (F + k)·v
(module
  (memory (export "memory") 1)
  (func (export "rates")
    (param $n i32) (param $u i32) (param $v i32) (param $du i32) (param $dv i32)
    (param $inv f64) (param $Du f64) (param $Dv f64) (param $r f64) (param $F f64) (param $Fk f64)
    (local $iy i32) (local $ix i32) (local $row i32) (local $k i32)
    (local $w i32) (local $e i32) (local $s i32) (local $no i32)
    (local $uc f64) (local $vc f64) (local $lu f64) (local $lv f64) (local $re f64)
    (block $done
      (loop $rows
        (br_if $done (i32.ge_s (local.get $iy) (local.get $n)))
        (local.set $row (i32.mul (local.get $iy) (local.get $n)))
        ;; The rows before and after, wrapped.
        (local.set $s
          (select
            (i32.mul (i32.sub (local.get $n) (i32.const 1)) (local.get $n))
            (i32.sub (local.get $row) (local.get $n))
            (i32.eqz (local.get $iy))))
        (local.set $no
          (select
            (i32.const 0)
            (i32.add (local.get $row) (local.get $n))
            (i32.eq (local.get $iy) (i32.sub (local.get $n) (i32.const 1)))))
        (local.set $ix (i32.const 0))
        (block $row_done
          (loop $cells
            (br_if $row_done (i32.ge_s (local.get $ix) (local.get $n)))
            (local.set $k (i32.add (local.get $row) (local.get $ix)))
            (local.set $w
              (select
                (i32.add (local.get $row) (i32.sub (local.get $n) (i32.const 1)))
                (i32.sub (local.get $k) (i32.const 1))
                (i32.eqz (local.get $ix))))
            (local.set $e
              (select
                (local.get $row)
                (i32.add (local.get $k) (i32.const 1))
                (i32.eq (local.get $ix) (i32.sub (local.get $n) (i32.const 1)))))
            (local.set $uc (f64.load (i32.add (local.get $u) (i32.shl (local.get $k) (i32.const 3)))))
            (local.set $vc (f64.load (i32.add (local.get $v) (i32.shl (local.get $k) (i32.const 3)))))
            (local.set $lu
              (f64.add
                (f64.mul
                  (f64.add
                    (f64.sub
                      (f64.load (i32.add (local.get $u) (i32.shl (local.get $w) (i32.const 3))))
                      (f64.mul (f64.const 2) (local.get $uc)))
                    (f64.load (i32.add (local.get $u) (i32.shl (local.get $e) (i32.const 3)))))
                  (local.get $inv))
                (f64.mul
                  (f64.add
                    (f64.sub
                      (f64.load (i32.add (local.get $u) (i32.shl (i32.add (local.get $s) (local.get $ix)) (i32.const 3))))
                      (f64.mul (f64.const 2) (local.get $uc)))
                    (f64.load (i32.add (local.get $u) (i32.shl (i32.add (local.get $no) (local.get $ix)) (i32.const 3)))))
                  (local.get $inv))))
            (local.set $lv
              (f64.add
                (f64.mul
                  (f64.add
                    (f64.sub
                      (f64.load (i32.add (local.get $v) (i32.shl (local.get $w) (i32.const 3))))
                      (f64.mul (f64.const 2) (local.get $vc)))
                    (f64.load (i32.add (local.get $v) (i32.shl (local.get $e) (i32.const 3)))))
                  (local.get $inv))
                (f64.mul
                  (f64.add
                    (f64.sub
                      (f64.load (i32.add (local.get $v) (i32.shl (i32.add (local.get $s) (local.get $ix)) (i32.const 3))))
                      (f64.mul (f64.const 2) (local.get $vc)))
                    (f64.load (i32.add (local.get $v) (i32.shl (i32.add (local.get $no) (local.get $ix)) (i32.const 3)))))
                  (local.get $inv))))
            (local.set $re
              (f64.mul (f64.mul (local.get $r) (local.get $uc)) (f64.mul (local.get $vc) (local.get $vc))))
            (f64.store (i32.add (local.get $du) (i32.shl (local.get $k) (i32.const 3)))
              (f64.add
                (f64.sub (f64.mul (local.get $Du) (local.get $lu)) (local.get $re))
                (f64.mul (local.get $F) (f64.sub (f64.const 1) (local.get $uc)))))
            (f64.store (i32.add (local.get $dv) (i32.shl (local.get $k) (i32.const 3)))
              (f64.sub
                (f64.add (f64.mul (local.get $Dv) (local.get $lv)) (local.get $re))
                (f64.mul (local.get $Fk) (local.get $vc))))
            (local.set $ix (i32.add (local.get $ix) (i32.const 1)))
            (br $cells)))
        (local.set $iy (i32.add (local.get $iy) (i32.const 1)))
        (br $rows)))))
