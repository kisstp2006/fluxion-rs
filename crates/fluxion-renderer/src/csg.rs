// ============================================================
// csg.rs — BSP-based Constructive Solid Geometry
//
// Port of the classic CSG.js algorithm (Evan Wallace, MIT).
// Represents solids as BSP trees of convex polygons and
// supports union, subtract, and intersect boolean operations.
//
// Usage:
//   let a = Solid::from_triangles(&verts_a, &idx_a);
//   let b = Solid::from_triangles(&verts_b, &idx_b).translate(offset);
//   let result = a.subtract(&b);
//   let (verts, indices) = result.to_triangles();
// ============================================================

use glam::Vec3;
use crate::mesh::Vertex;

const EPS: f32 = 1e-5;

// ── Vertex ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct CsgVert {
    pos:    Vec3,
    normal: Vec3,
    uv:     [f32; 2],
}

impl CsgVert {
    fn lerp(&self, other: &CsgVert, t: f32) -> CsgVert {
        CsgVert {
            pos:    self.pos.lerp(other.pos, t),
            normal: self.normal.lerp(other.normal, t).normalize_or_zero(),
            uv:     [
                self.uv[0] + t * (other.uv[0] - self.uv[0]),
                self.uv[1] + t * (other.uv[1] - self.uv[1]),
            ],
        }
    }
}

// ── Polygon ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Poly {
    verts: Vec<CsgVert>,
    n:     Vec3,   // plane normal
    w:     f32,    // plane offset: n · p == w for all p on the plane
}

impl Poly {
    fn new(verts: Vec<CsgVert>) -> Self {
        let n = if verts.len() >= 3 {
            (verts[1].pos - verts[0].pos)
                .cross(verts[2].pos - verts[0].pos)
                .normalize_or_zero()
        } else {
            Vec3::Y
        };
        let w = if n.length_squared() > EPS { n.dot(verts[0].pos) } else { 0.0 };
        Poly { verts, n, w }
    }

    fn flipped(&self) -> Poly {
        let mut p = self.clone();
        p.verts.reverse();
        p.n = -p.n;
        p.w = -p.w;
        p
    }
}

// ── Plane split ──────────────────────────────────────────────────────────────

const COPLANAR: u8 = 0;
const FRONT:    u8 = 1;
const BACK:     u8 = 2;
const SPANNING: u8 = 3;

fn split_poly(
    poly:  &Poly,
    pn:    Vec3,
    pw:    f32,
    cf:    &mut Vec<Poly>,
    cb:    &mut Vec<Poly>,
    front: &mut Vec<Poly>,
    back:  &mut Vec<Poly>,
) {
    let mut ptype = 0u8;
    let types: Vec<u8> = poly.verts.iter().map(|v| {
        let t = pn.dot(v.pos) - pw;
        let vt = if t < -EPS { BACK } else if t > EPS { FRONT } else { COPLANAR };
        ptype |= vt;
        vt
    }).collect();

    match ptype {
        COPLANAR => {
            if pn.dot(poly.n) > 0.0 { cf.push(poly.clone()); } else { cb.push(poly.clone()); }
        }
        FRONT => front.push(poly.clone()),
        BACK  => back.push(poly.clone()),
        _ /* SPANNING */ => {
            let mut fv = Vec::new();
            let mut bv = Vec::new();
            let nv = poly.verts.len();
            for i in 0..nv {
                let j  = (i + 1) % nv;
                let ti = types[i];
                let tj = types[j];
                let vi = &poly.verts[i];
                let vj = &poly.verts[j];
                if ti != BACK  { fv.push(vi.clone()); }
                if ti != FRONT { bv.push(vi.clone()); }
                if (ti | tj) == SPANNING {
                    let denom = pn.dot(vj.pos - vi.pos);
                    if denom.abs() > EPS {
                        let t = (pw - pn.dot(vi.pos)) / denom;
                        let v = vi.lerp(vj, t);
                        fv.push(v.clone());
                        bv.push(v);
                    }
                }
            }
            if fv.len() >= 3 { front.push(Poly::new(fv)); }
            if bv.len() >= 3 { back.push(Poly::new(bv));  }
        }
    }
}

// ── BSP Node ─────────────────────────────────────────────────────────────────

struct Node {
    plane: Option<(Vec3, f32)>,   // (normal, offset) — None for empty leaf
    front: Option<Box<Node>>,
    back:  Option<Box<Node>>,
    polys: Vec<Poly>,
}

impl Node {
    fn empty() -> Self {
        Node { plane: None, front: None, back: None, polys: Vec::new() }
    }

    fn from_polys(polys: Vec<Poly>) -> Self {
        let mut n = Node::empty();
        n.build(polys);
        n
    }

    fn build(&mut self, polys: Vec<Poly>) {
        if polys.is_empty() { return; }

        // Choose splitting plane (first polygon's plane if none set yet).
        let (pn, pw) = *self.plane.get_or_insert((polys[0].n, polys[0].w));

        let mut front = Vec::new();
        let mut back  = Vec::new();
        let mut cf    = Vec::new();
        let mut cb    = Vec::new();

        for poly in polys {
            split_poly(&poly, pn, pw, &mut cf, &mut cb, &mut front, &mut back);
        }
        self.polys.extend(cf);
        self.polys.extend(cb);

        if !front.is_empty() {
            let node = self.front.get_or_insert_with(|| Box::new(Node::empty()));
            node.build(front);
        }
        if !back.is_empty() {
            let node = self.back.get_or_insert_with(|| Box::new(Node::empty()));
            node.build(back);
        }
    }

    fn all_polys(&self) -> Vec<Poly> {
        let mut result = self.polys.clone();
        if let Some(f) = &self.front { result.extend(f.all_polys()); }
        if let Some(b) = &self.back  { result.extend(b.all_polys()); }
        result
    }

    /// Keep only polygons on the FRONT side of this BSP tree.
    fn clip_polys(&self, polys: Vec<Poly>) -> Vec<Poly> {
        let (pn, pw) = match self.plane {
            None => return polys, // no plane → pass all through
            Some(p) => p,
        };

        let mut fvec = Vec::new();
        let mut bvec = Vec::new();
        let mut cf   = Vec::new();
        let mut cb   = Vec::new();

        for poly in polys {
            split_poly(&poly, pn, pw, &mut cf, &mut cb, &mut fvec, &mut bvec);
        }
        fvec.extend(cf);
        bvec.extend(cb);

        let mut front = if let Some(f) = &self.front { f.clip_polys(fvec) } else { fvec };
        let     back  = if let Some(b) = &self.back  { b.clip_polys(bvec) } else { Vec::new() };

        front.extend(back);
        front
    }

    /// Clip this node's polygons to the inside of `bsp`.
    fn clip_to(&mut self, bsp: &Node) {
        self.polys = bsp.clip_polys(std::mem::take(&mut self.polys));
        if let Some(f) = &mut self.front { f.clip_to(bsp); }
        if let Some(b) = &mut self.back  { b.clip_to(bsp); }
    }

    fn invert(&mut self) {
        self.polys = self.polys.iter().map(|p| p.flipped()).collect();
        if let Some((ref mut n, ref mut w)) = self.plane {
            *n = -*n;
            *w = -*w;
        }
        std::mem::swap(&mut self.front, &mut self.back);
        if let Some(f) = &mut self.front { f.invert(); }
        if let Some(b) = &mut self.back  { b.invert(); }
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// A CSG solid represented as a collection of convex polygons.
pub struct Solid {
    polys: Vec<Poly>,
}

impl Solid {
    /// Build a CSG solid from an indexed triangle mesh.
    pub fn from_triangles(vertices: &[Vertex], indices: &[u32]) -> Self {
        let mut polys = Vec::with_capacity(indices.len() / 3);
        for tri in indices.chunks_exact(3) {
            let va = &vertices[tri[0] as usize];
            let vb = &vertices[tri[1] as usize];
            let vc = &vertices[tri[2] as usize];

            let verts = vec![
                CsgVert { pos: Vec3::from(va.position), normal: Vec3::from(va.normal), uv: va.uv },
                CsgVert { pos: Vec3::from(vb.position), normal: Vec3::from(vb.normal), uv: vb.uv },
                CsgVert { pos: Vec3::from(vc.position), normal: Vec3::from(vc.normal), uv: vc.uv },
            ];
            let poly = Poly::new(verts);
            // Skip degenerate (zero-area) triangles.
            if poly.n.length_squared() > EPS {
                polys.push(poly);
            }
        }
        Solid { polys }
    }

    /// Convert this solid back to an indexed triangle mesh.
    /// Polygons may have > 3 verts after clipping; fan-triangulation is used.
    pub fn to_triangles(&self) -> (Vec<Vertex>, Vec<u32>) {
        let mut vertices: Vec<Vertex> = Vec::new();
        let mut indices:  Vec<u32>    = Vec::new();

        for poly in &self.polys {
            if poly.verts.len() < 3 { continue; }
            let base = vertices.len() as u32;
            for v in &poly.verts {
                vertices.push(Vertex {
                    position: v.pos.to_array(),
                    normal:   v.normal.to_array(),
                    tangent:  [1.0, 0.0, 0.0, 1.0],
                    uv:       v.uv,
                });
            }
            for i in 1..((poly.verts.len() as u32) - 1) {
                indices.extend_from_slice(&[base, base + i, base + i + 1]);
            }
        }

        (vertices, indices)
    }

    /// Translate all vertex positions by `offset`.
    /// Used to position child CSG solids relative to the parent.
    pub fn translate(mut self, offset: Vec3) -> Self {
        for poly in &mut self.polys {
            for v in &mut poly.verts {
                v.pos += offset;
            }
            // Recompute plane offset after translation.
            if !poly.verts.is_empty() {
                poly.w = poly.n.dot(poly.verts[0].pos);
            }
        }
        self
    }

    // ── Boolean operations (CSG.js algorithm) ─────────────────────────────────

    pub fn union(&self, other: &Solid) -> Solid {
        let mut a = Node::from_polys(self.polys.clone());
        let mut b = Node::from_polys(other.polys.clone());
        a.clip_to(&b);
        b.clip_to(&a);
        b.invert();
        b.clip_to(&a);
        b.invert();
        let mut all = a.all_polys();
        all.extend(b.all_polys());
        Solid { polys: Node::from_polys(all).all_polys() }
    }

    pub fn subtract(&self, other: &Solid) -> Solid {
        let mut a = Node::from_polys(self.polys.clone());
        let mut b = Node::from_polys(other.polys.clone());
        a.invert();
        a.clip_to(&b);
        b.clip_to(&a);
        b.invert();
        b.clip_to(&a);
        b.invert();
        let mut all = a.all_polys();
        all.extend(b.all_polys());
        let mut n = Node::from_polys(all);
        n.invert();
        Solid { polys: n.all_polys() }
    }

    pub fn intersect(&self, other: &Solid) -> Solid {
        let mut a = Node::from_polys(self.polys.clone());
        let mut b = Node::from_polys(other.polys.clone());
        a.invert();
        b.clip_to(&a);
        b.invert();
        a.clip_to(&b);
        b.clip_to(&a);
        let mut all = a.all_polys();
        all.extend(b.all_polys());
        let mut n = Node::from_polys(all);
        n.invert();
        Solid { polys: n.all_polys() }
    }
}
