#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use egui::{Color32, CursorIcon, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2};
use image::RgbaImage;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

const A4_W_MM: f32 = 210.0;
const A4_H_MM: f32 = 297.0;
const MIN_SIZE_MM: f32 = 10.0;
const PREVIEW_MAX_PX: u32 = 3500;
const MIN_CROP: f32 = 0.05;

/// Retângulo em milímetros, relativo ao canto superior esquerdo da folha A4.
#[derive(Clone, Copy, Debug)]
struct RectMm {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl RectMm {
    fn left(&self) -> f32 {
        self.x
    }
    fn top(&self) -> f32 {
        self.y
    }
    fn right(&self) -> f32 {
        self.x + self.w
    }
    fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Handle {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

impl Handle {
    fn moves_left(self) -> bool {
        matches!(self, Handle::W | Handle::NW | Handle::SW)
    }
    fn moves_right(self) -> bool {
        matches!(self, Handle::E | Handle::NE | Handle::SE)
    }
    fn moves_top(self) -> bool {
        matches!(self, Handle::N | Handle::NW | Handle::NE)
    }
    fn moves_bottom(self) -> bool {
        matches!(self, Handle::S | Handle::SW | Handle::SE)
    }
    fn is_corner(self) -> bool {
        matches!(self, Handle::NE | Handle::NW | Handle::SE | Handle::SW)
    }
    fn cursor(self) -> CursorIcon {
        match self {
            Handle::N | Handle::S => CursorIcon::ResizeVertical,
            Handle::E | Handle::W => CursorIcon::ResizeHorizontal,
            Handle::NW | Handle::SE => CursorIcon::ResizeNwSe,
            Handle::NE | Handle::SW => CursorIcon::ResizeNeSw,
        }
    }
}

#[derive(Clone, Copy)]
enum DragMode {
    Move { grab: Vec2 },
    Resize(Handle),
}

/// Uma foto colocada na página: imagem em resolução original, textura de
/// pré-visualização, retângulo de destino (mm na folha) e recorte (uv 0..1).
struct Photo {
    img: Arc<RgbaImage>,
    texture: TextureHandle,
    tex_name: String,
    img_aspect: f32,
    dest: RectMm,
    crop: Rect,
    name: String,
    /// Célula da colagem automática (None se a foto foi movida manualmente).
    cell: Option<RectMm>,
    /// Tamanho físico (mm) da imagem inteira: página do PDF ou DPI do arquivo.
    phys_mm: (f32, f32),
}

impl Photo {
    /// Proporção (largura/altura) da região recortada, em pixels.
    fn crop_aspect(&self) -> f32 {
        let w = self.crop.width() * self.img.width() as f32;
        let h = (self.crop.height() * self.img.height() as f32).max(1.0);
        (w / h).max(0.01)
    }
}

struct App {
    photos: Vec<Photo>,
    selected: Option<usize>,
    next_id: u64,
    /// Bordas em mm: [esquerda, superior, direita, inferior]
    margins: [f32; 4],
    landscape: bool,
    /// Tamanho real (PDF na dimensão da página / foto pelo DPI) em vez de caber na folha
    real_size: bool,
    keep_aspect: bool,
    black_frame: bool,
    frame_mm: f32,
    crop_mode: bool,
    drag: Option<DragMode>,
    crop_drag: Option<DragMode>,
    status: String,
    /// Último carregamento falhou por falta do codec HEVC (HEIC do iPhone)
    hevc_missing: bool,
    /// PDF carregado: caminho, página atual (base 0), total e foto vinculada
    pdf_path: Option<PathBuf>,
    pdf_page: u32,
    pdf_page_count: u32,
    pdf_photo: Option<usize>,
    print_rx: Option<Receiver<String>>,
    /// Linhas-guia de ancoragem desenhadas neste frame (valores em mm)
    snap_x: Vec<f32>,
    snap_y: Vec<f32>,
    /// Geometria do canvas no último frame (para mapear drops do Explorer)
    view_origin: Pos2,
    view_scale: f32,
}

impl Default for App {
    fn default() -> Self {
        Self {
            photos: Vec::new(),
            selected: None,
            next_id: 0,
            margins: [5.0, 5.0, 5.0, 5.0],
            landscape: false,
            real_size: true,
            keep_aspect: false,
            black_frame: true,
            frame_mm: 2.0,
            crop_mode: false,
            drag: None,
            crop_drag: None,
            status: "Adicione uma ou mais fotos para começar (ou arraste do Explorer).".into(),
            hevc_missing: false,
            pdf_path: None,
            pdf_page: 0,
            pdf_page_count: 0,
            pdf_photo: None,
            print_rx: None,
            snap_x: Vec::new(),
            snap_y: Vec::new(),
            view_origin: Pos2::ZERO,
            view_scale: 0.0,
        }
    }
}

impl App {
    fn page_size(&self) -> (f32, f32) {
        if self.landscape {
            (A4_H_MM, A4_W_MM)
        } else {
            (A4_W_MM, A4_H_MM)
        }
    }

    fn print_area(&self) -> RectMm {
        let (pw, ph) = self.page_size();
        let [l, t, r, b] = self.margins;
        RectMm {
            x: l,
            y: t,
            w: (pw - l - r).max(1.0),
            h: (ph - t - b).max(1.0),
        }
    }

    fn load_image(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.hevc_missing = false;
        let is_pdf = path
            .extension()
            .map(|e| e.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false);
        if is_pdf {
            self.open_pdf(ctx, path);
            return;
        }

        match decode_image(&path) {
            Ok(rgba) => {
                let name = file_name(&path);
                let phys = image_phys_mm(&path, rgba.width(), rgba.height());
                self.add_photo(ctx, rgba, name.clone(), phys);
                self.status = format!("Foto adicionada: {name}");
            }
            Err(e) => {
                self.hevc_missing = e.contains("codec HEVC");
                self.status = format!("Erro ao abrir a imagem: {e}");
            }
        }
    }

    fn open_pdf(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.pdf_path = Some(path.clone());
        self.pdf_page = 0;
        match render_pdf_page(&path, 0) {
            Ok((rgba, count, phys)) => {
                self.pdf_page_count = count;
                let base = file_name(&path);
                let idx = self.add_photo(ctx, rgba, format!("{base} (p.1)"), phys);
                self.pdf_photo = Some(idx);
                self.status = format!("PDF: {base} — página 1 de {count}");
            }
            Err(e) => {
                self.pdf_page_count = 0;
                self.pdf_photo = None;
                self.status = format!("Erro ao abrir PDF: {e}");
            }
        }
    }

    /// Troca a página do PDF vinculado, substituindo a imagem da foto ligada a
    /// ele e preservando posição, tamanho e recorte já ajustados.
    fn change_pdf_page(&mut self, ctx: &egui::Context) {
        let Some(idx) = self.pdf_photo else { return };
        let Some(path) = self.pdf_path.clone() else { return };
        match render_pdf_page(&path, self.pdf_page) {
            Ok((rgba, count, phys)) => {
                self.pdf_page_count = count;
                self.pdf_page = self.pdf_page.min(count.saturating_sub(1));
                let name = format!("{} (p.{})", file_name(&path), self.pdf_page + 1);
                let aspect = rgba.width() as f32 / rgba.height().max(1) as f32;
                if let Some(ph) = self.photos.get_mut(idx) {
                    ph.img_aspect = aspect;
                    ph.texture = make_texture(ctx, &ph.tex_name, &rgba);
                    ph.img = Arc::new(rgba);
                    ph.name = name;
                    ph.phys_mm = phys;
                }
                if self.auto_orient(aspect) {
                    self.apply_size_mode();
                }
                self.status = format!("PDF: página {} de {}", self.pdf_page + 1, count);
            }
            Err(e) => self.status = format!("Erro no PDF: {e}"),
        }
    }

    /// Adiciona uma nova foto. Em "tamanho real" ela é centralizada na área de
    /// impressão com sua dimensão física; em "caber na folha" todas são
    /// reorganizadas em colagem preenchendo a folha.
    fn add_photo(
        &mut self,
        ctx: &egui::Context,
        img: RgbaImage,
        name: String,
        phys_mm: (f32, f32),
    ) -> usize {
        let aspect = img.width() as f32 / img.height().max(1) as f32;
        let id = self.next_id;
        self.next_id += 1;
        let tex_name = format!("photo_{id}");
        let texture = make_texture(ctx, &tex_name, &img);
        let reoriented = self.auto_orient(aspect);

        let area = self.print_area();
        self.photos.push(Photo {
            img: Arc::new(img),
            texture,
            tex_name,
            img_aspect: aspect,
            dest: fit_rect(area, aspect),
            crop: full_crop(),
            name,
            cell: None,
            phys_mm,
        });
        let idx = self.photos.len() - 1;
        self.selected = Some(idx);
        self.crop_mode = false;
        if reoriented {
            // A folha mudou de orientação: reposiciona todas as fotos
            self.apply_size_mode();
        } else if self.real_size {
            self.place_real(idx);
        } else {
            self.auto_arrange();
        }
        idx
    }

    /// Orientação automática: arquivo em paisagem vira a folha para paisagem,
    /// em retrato para retrato (a escolha manual continua disponível no painel).
    /// Devolve true se a orientação mudou.
    fn auto_orient(&mut self, aspect: f32) -> bool {
        let want = aspect > 1.0;
        if want != self.landscape {
            self.landscape = want;
            true
        } else {
            false
        }
    }

    /// Retângulo da foto `idx` no tamanho físico real (considerando o recorte),
    /// centralizado na área de impressão.
    fn real_rect(&self, idx: usize) -> Option<RectMm> {
        let ph = self.photos.get(idx)?;
        let area = self.print_area();
        let mut w = (ph.phys_mm.0 * ph.crop.width()).max(MIN_SIZE_MM);
        let mut h = (ph.phys_mm.1 * ph.crop.height()).max(MIN_SIZE_MM);
        // Maior que a área de impressão: reduz mantendo a proporção, para a
        // foto inteira aparecer em vez de ser cortada pelas bordas
        let k = (area.w / w).min(area.h / h);
        if k < 1.0 {
            w *= k;
            h *= k;
        }
        Some(RectMm {
            x: area.x + (area.w - w) / 2.0,
            y: area.y + (area.h - h) / 2.0,
            w,
            h,
        })
    }

    fn place_real(&mut self, idx: usize) {
        if let Some(r) = self.real_rect(idx) {
            let ph = &mut self.photos[idx];
            ph.dest = r;
            ph.cell = None;
        }
    }

    /// Aplica o modo de tamanho escolhido a todas as fotos.
    fn apply_size_mode(&mut self) {
        if self.real_size {
            for i in 0..self.photos.len() {
                self.place_real(i);
            }
        } else {
            self.auto_arrange();
        }
    }

    /// Monta uma colagem automática estilo editor de fotos: divide a área em
    /// células (uma foto grande + outras menores, conforme a quantidade),
    /// preenchendo toda a folha. Cada foto é encaixada cobrindo sua célula.
    fn auto_arrange(&mut self) {
        let n = self.photos.len();
        if n == 0 {
            return;
        }
        let area = self.print_area();
        let cells = template_cells(n, area);

        // Atribui as células às fotos por proporção (célula maior primeiro pega
        // a foto cuja proporção melhor a preenche)
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| {
            let area_a = cells[a].w * cells[a].h;
            let area_b = cells[b].w * cells[b].h;
            area_b
                .partial_cmp(&area_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut used = vec![false; n];
        let mut assign = vec![0usize; n];
        for &ci in &order {
            let ca = (cells[ci].w / cells[ci].h).max(0.001);
            let mut best: Option<(usize, f32)> = None;
            for p in 0..n {
                if !used[p] {
                    let d = (self.photos[p].img_aspect / ca).ln().abs();
                    if best.map_or(true, |(_, bd)| d < bd) {
                        best = Some((p, d));
                    }
                }
            }
            if let Some((p, _)) = best {
                used[p] = true;
                assign[p] = ci;
            }
        }
        for p in 0..n {
            self.photos[p].cell = Some(cells[assign[p]]);
        }
        self.reflow_cells();
    }

    /// Reaplica as células às fotos da colagem, recuando cada foto pela
    /// espessura da moldura para que a borda preta fique uniforme entre elas.
    fn reflow_cells(&mut self) {
        let m = if self.black_frame {
            self.frame_mm.max(0.0)
        } else {
            0.0
        };
        for ph in &mut self.photos {
            if let Some(cell) = ph.cell {
                let d = RectMm {
                    x: cell.x + m,
                    y: cell.y + m,
                    w: (cell.w - 2.0 * m).max(MIN_SIZE_MM),
                    h: (cell.h - 2.0 * m).max(MIN_SIZE_MM),
                };
                // Encaixa a foto inteira na célula sem cortar (mantém o
                // recorte que o usuário já tiver feito)
                ph.dest = fit_rect(d, ph.crop_aspect());
            }
        }
    }

    /// Foto (índice) sob uma posição de tela, usando a geometria do último frame.
    /// Devolve None em modo recorte ou se o ponto estiver fora de qualquer foto.
    fn photo_at_screen(&self, pos: Option<Pos2>) -> Option<usize> {
        if self.crop_mode || self.view_scale <= 0.0 {
            return None;
        }
        let p = pos?;
        let mm = Pos2::new(
            (p.x - self.view_origin.x) / self.view_scale,
            (p.y - self.view_origin.y) / self.view_scale,
        );
        (0..self.photos.len()).rev().find(|&i| {
            let d = self.photos[i].dest;
            mm.x >= d.left() && mm.x <= d.right() && mm.y >= d.top() && mm.y <= d.bottom()
        })
    }

    /// Substitui a imagem da foto `idx` mantendo o mesmo espaço na folha
    /// (a nova imagem é encaixada nesse espaço sem distorcer).
    fn replace_photo(&mut self, ctx: &egui::Context, idx: usize, path: PathBuf) {
        self.hevc_missing = false;
        if idx >= self.photos.len() {
            self.load_image(ctx, path);
            return;
        }
        let is_pdf = path
            .extension()
            .map(|e| e.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false);

        if is_pdf {
            match render_pdf_page(&path, 0) {
                Ok((rgba, count, phys)) => {
                    let aspect = rgba.width() as f32 / rgba.height().max(1) as f32;
                    let name = format!("{} (p.1)", file_name(&path));
                    if let Some(ph) = self.photos.get_mut(idx) {
                        let old = ph.dest;
                        ph.img_aspect = aspect;
                        ph.texture = make_texture(ctx, &ph.tex_name, &rgba);
                        ph.img = Arc::new(rgba);
                        ph.crop = full_crop();
                        ph.dest = fit_rect(old, aspect);
                        ph.name = name;
                        ph.phys_mm = phys;
                    }
                    if self.auto_orient(aspect) {
                        self.apply_size_mode();
                    } else if self.real_size {
                        self.place_real(idx);
                    }
                    self.pdf_path = Some(path);
                    self.pdf_page = 0;
                    self.pdf_page_count = count;
                    self.pdf_photo = Some(idx);
                    self.selected = Some(idx);
                    self.crop_mode = false;
                    self.reflow_cells();
                    self.status = format!("Foto substituída (PDF, {count} pág.)");
                }
                Err(e) => self.status = format!("Erro ao abrir PDF: {e}"),
            }
            return;
        }

        match decode_image(&path) {
            Ok(rgba) => {
                let aspect = rgba.width() as f32 / rgba.height().max(1) as f32;
                let name = file_name(&path);
                let phys = image_phys_mm(&path, rgba.width(), rgba.height());
                if let Some(ph) = self.photos.get_mut(idx) {
                    let old = ph.dest;
                    ph.img_aspect = aspect;
                    ph.texture = make_texture(ctx, &ph.tex_name, &rgba);
                    ph.img = Arc::new(rgba);
                    ph.crop = full_crop();
                    ph.dest = fit_rect(old, aspect);
                    ph.name = name.clone();
                    ph.phys_mm = phys;
                }
                if self.auto_orient(aspect) {
                    self.apply_size_mode();
                } else if self.real_size {
                    self.place_real(idx);
                }
                if self.pdf_photo == Some(idx) {
                    self.pdf_photo = None;
                    self.pdf_page_count = 0;
                }
                self.selected = Some(idx);
                self.crop_mode = false;
                self.reflow_cells();
                self.status = format!("Foto substituída: {name}");
            }
            Err(e) => {
                self.hevc_missing = e.contains("codec HEVC");
                self.status = format!("Erro ao abrir a imagem: {e}");
            }
        }
    }

    fn remove_selected(&mut self) {
        let Some(i) = self.selected else { return };
        if i >= self.photos.len() {
            return;
        }
        self.photos.remove(i);
        match self.pdf_photo {
            Some(p) if p == i => {
                self.pdf_photo = None;
                self.pdf_page_count = 0;
            }
            Some(p) if p > i => self.pdf_photo = Some(p - 1),
            _ => {}
        }
        self.crop_mode = false;
        self.selected = if self.photos.is_empty() {
            None
        } else {
            Some(i.min(self.photos.len() - 1))
        };
    }

    fn bring_to_front(&mut self) {
        let Some(i) = self.selected else { return };
        if i >= self.photos.len() {
            return;
        }
        let ph = self.photos.remove(i);
        self.photos.push(ph);
        let new_idx = self.photos.len() - 1;
        match self.pdf_photo {
            Some(p) if p == i => self.pdf_photo = Some(new_idx),
            Some(p) if p > i => self.pdf_photo = Some(p - 1),
            _ => {}
        }
        self.selected = Some(new_idx);
    }

    fn fit_selected(&mut self, keep_aspect: bool) {
        let area = self.print_area();
        if let Some(i) = self.selected {
            if let Some(ph) = self.photos.get_mut(i) {
                ph.dest = if keep_aspect {
                    fit_rect(area, ph.crop_aspect())
                } else {
                    area
                };
            }
        }
    }

    fn center_selected(&mut self) {
        let area = self.print_area();
        if let Some(i) = self.selected {
            if let Some(ph) = self.photos.get_mut(i) {
                ph.dest.x = area.x + (area.w - ph.dest.w) / 2.0;
                ph.dest.y = area.y + (area.h - ph.dest.h) / 2.0;
            }
        }
    }

    fn reset_crop(&mut self) {
        if let Some(i) = self.selected {
            if let Some(ph) = self.photos.get_mut(i) {
                ph.crop = full_crop();
            }
        }
    }

    /// Sai do modo recorte ajustando a altura da moldura à proporção recortada,
    /// para que a parte mantida não fique distorcida.
    fn finish_crop(&mut self) {
        self.crop_mode = false;
        self.crop_drag = None;
        if let Some(i) = self.selected {
            if let Some(ph) = self.photos.get_mut(i) {
                let a = ph.crop_aspect();
                ph.dest.h = (ph.dest.w / a).max(MIN_SIZE_MM);
            }
        }
    }

    /// Alvos de ancoragem: bordas da folha, bordas da área de impressão e
    /// bordas (com moldura) das demais fotos — para alinhar uma foto às outras.
    fn snap_targets(&self) -> (Vec<f32>, Vec<f32>) {
        let (pw, ph) = self.page_size();
        let a = self.print_area();
        let mut xs = vec![a.left(), a.right()];
        let mut ys = vec![a.top(), a.bottom()];
        if a.left() > 0.05 {
            xs.push(0.0);
        }
        if a.right() < pw - 0.05 {
            xs.push(pw);
        }
        if a.top() > 0.05 {
            ys.push(0.0);
        }
        if a.bottom() < ph - 0.05 {
            ys.push(ph);
        }

        // Bordas das outras fotos (alinhamento foto a foto)
        let m = if self.black_frame { self.frame_mm } else { 0.0 };
        for (i, photo) in self.photos.iter().enumerate() {
            if Some(i) == self.selected {
                continue;
            }
            let f = frame_of(photo.dest, m);
            xs.push(f.left());
            xs.push(f.right());
            ys.push(f.top());
            ys.push(f.bottom());
        }
        (xs, ys)
    }

    /// `m` = espessura da moldura: a ancoragem usa a borda externa (preta).
    fn move_rect(&mut self, mut r: RectMm, p_mm: Pos2, grab: Vec2, thr: f32, m: f32) -> RectMm {
        r.x = p_mm.x - grab.x;
        r.y = p_mm.y - grab.y;
        let (tx, ty) = self.snap_targets();

        let mut best: Option<(f32, f32, f32)> = None; // (|delta|, delta, alvo)
        for &t in &tx {
            for d in [t - (r.left() - m), t - (r.right() + m)] {
                if d.abs() < thr && best.map_or(true, |b| d.abs() < b.0) {
                    best = Some((d.abs(), d, t));
                }
            }
        }
        if let Some((_, d, t)) = best {
            r.x += d;
            self.snap_x.push(t);
        }

        let mut best: Option<(f32, f32, f32)> = None;
        for &t in &ty {
            for d in [t - (r.top() - m), t - (r.bottom() + m)] {
                if d.abs() < thr && best.map_or(true, |b| d.abs() < b.0) {
                    best = Some((d.abs(), d, t));
                }
            }
        }
        if let Some((_, d, t)) = best {
            r.y += d;
            self.snap_y.push(t);
        }
        r
    }

    /// `m` = espessura da moldura: a borda externa (preta) é que ancora.
    fn resize_rect(
        &mut self,
        r: RectMm,
        keep_aspect: Option<f32>,
        handle: Handle,
        p_mm: Pos2,
        thr: f32,
        m: f32,
    ) -> RectMm {
        let (tx, ty) = self.snap_targets();
        let snap = |v: f32, targets: &[f32]| -> (f32, Option<f32>) {
            let mut best: Option<(f32, f32)> = None;
            for &t in targets {
                let d = (t - v).abs();
                if d < thr && best.map_or(true, |b| d < b.0) {
                    best = Some((d, t));
                }
            }
            match best {
                Some((_, t)) => (t, Some(t)),
                None => (v, None),
            }
        };

        let (mut left, mut right, mut top, mut bottom) =
            (r.left(), r.right(), r.top(), r.bottom());

        if handle.moves_left() {
            let (v, s) = snap(p_mm.x - m, &tx);
            left = (v + m).min(right - MIN_SIZE_MM);
            if let Some(t) = s {
                self.snap_x.push(t);
            }
        }
        if handle.moves_right() {
            let (v, s) = snap(p_mm.x + m, &tx);
            right = (v - m).max(left + MIN_SIZE_MM);
            if let Some(t) = s {
                self.snap_x.push(t);
            }
        }
        if handle.moves_top() {
            let (v, s) = snap(p_mm.y - m, &ty);
            top = (v + m).min(bottom - MIN_SIZE_MM);
            if let Some(t) = s {
                self.snap_y.push(t);
            }
        }
        if handle.moves_bottom() {
            let (v, s) = snap(p_mm.y + m, &ty);
            bottom = (v - m).max(top + MIN_SIZE_MM);
            if let Some(t) = s {
                self.snap_y.push(t);
            }
        }

        // Cantos com proporção travada: o lado dominante manda, o outro acompanha
        if let Some(aspect) = keep_aspect {
            if handle.is_corner() && aspect > 0.0 {
                let w = right - left;
                let h = bottom - top;
                if w / aspect >= h {
                    let nh = (w / aspect).max(MIN_SIZE_MM);
                    if handle.moves_top() {
                        top = bottom - nh;
                    } else {
                        bottom = top + nh;
                    }
                } else {
                    let nw = (h * aspect).max(MIN_SIZE_MM);
                    if handle.moves_left() {
                        left = right - nw;
                    } else {
                        right = left + nw;
                    }
                }
            }
        }

        RectMm {
            x: left,
            y: top,
            w: right - left,
            h: bottom - top,
        }
    }

    fn start_print(&mut self) {
        if self.photos.is_empty() {
            self.status = "Adicione ao menos uma foto antes de imprimir.".into();
            return;
        }
        let items: Vec<(Arc<RgbaImage>, [f32; 4], RectMm)> = self
            .photos
            .iter()
            .map(|p| {
                (
                    p.img.clone(),
                    [p.crop.min.x, p.crop.min.y, p.crop.max.x, p.crop.max.y],
                    p.dest,
                )
            })
            .collect();
        let area = self.print_area();
        let landscape = self.landscape;
        let frame_mm = if self.black_frame { self.frame_mm } else { 0.0 };
        let owner = owner_window();
        let (tx, rx) = channel();
        self.print_rx = Some(rx);
        self.status = "Abrindo diálogo de impressão...".into();
        std::thread::spawn(move || {
            let msg = match print_photos(&items, area, landscape, frame_mm, owner) {
                Ok(true) => "Impressão enviada para a impressora.".to_string(),
                Ok(false) => "Impressão cancelada.".to_string(),
                Err(e) => format!("Erro na impressão: {e}"),
            };
            let _ = tx.send(msg);
        });
    }

    fn side_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("controles")
            .min_width(240.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.heading("Impressão de Foto A4");
                ui.add_space(8.0);

                if ui.button("➕  Adicionar arquivo(s)...").clicked() {
                    if let Some(paths) = rfd::FileDialog::new()
                        .add_filter(
                            "Imagens e PDF",
                            &[
                                "png", "jpg", "jpeg", "heic", "heif", "hif", "avif", "bmp",
                                "webp", "tif", "tiff", "gif", "pdf",
                            ],
                        )
                        .pick_files()
                    {
                        for p in paths {
                            self.load_image(ctx, p);
                        }
                    }
                }
                ui.label(
                    egui::RichText::new(
                        "Você pode adicionar várias fotos ou PDFs na mesma folha\n\
                         e arrastar arquivos do Explorer para a janela.",
                    )
                    .small()
                    .color(Color32::GRAY),
                );

                if self.pdf_photo.is_some() && self.pdf_page_count > 0 {
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!("PDF — {} página(s)", self.pdf_page_count))
                            .strong(),
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(self.pdf_page > 0, egui::Button::new("◀"))
                            .on_hover_text("Página anterior")
                            .clicked()
                        {
                            self.pdf_page -= 1;
                            self.change_pdf_page(ctx);
                        }
                        let mut page1 = self.pdf_page + 1;
                        if ui
                            .add(
                                egui::DragValue::new(&mut page1)
                                    .speed(0.2)
                                    .range(1..=self.pdf_page_count),
                            )
                            .changed()
                        {
                            self.pdf_page = page1.clamp(1, self.pdf_page_count) - 1;
                            self.change_pdf_page(ctx);
                        }
                        ui.label(format!("de {}", self.pdf_page_count));
                        if ui
                            .add_enabled(
                                self.pdf_page + 1 < self.pdf_page_count,
                                egui::Button::new("▶"),
                            )
                            .on_hover_text("Próxima página")
                            .clicked()
                        {
                            self.pdf_page += 1;
                            self.change_pdf_page(ctx);
                        }
                    });
                }

                ui.separator();
                ui.label(
                    egui::RichText::new(format!("Fotos na página: {}", self.photos.len())).strong(),
                );
                if !self.photos.is_empty() {
                    egui::ScrollArea::vertical()
                        .max_height(110.0)
                        .show(ui, |ui| {
                            let mut clicked = None;
                            for (i, ph) in self.photos.iter().enumerate() {
                                let sel = self.selected == Some(i);
                                if ui
                                    .selectable_label(sel, format!("{}. {}", i + 1, ph.name))
                                    .clicked()
                                {
                                    clicked = Some(i);
                                }
                            }
                            if let Some(i) = clicked {
                                self.selected = Some(i);
                                self.crop_mode = false;
                            }
                        });
                }

                let has_sel = self.selected.is_some();
                ui.add_enabled_ui(has_sel, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("✂  Recortar").clicked() {
                            self.crop_mode = true;
                        }
                        if ui.button("⛶  Preencher área").clicked() {
                            self.fit_selected(false);
                        }
                        if ui.button("🔍  Ajustar proporção").clicked() {
                            self.fit_selected(true);
                        }
                        if ui.button("🎯  Centralizar").clicked() {
                            self.center_selected();
                        }
                        if ui.button("⬆  Trazer p/ frente").clicked() {
                            self.bring_to_front();
                        }
                        if ui.button("🗑  Remover").clicked() {
                            self.remove_selected();
                        }
                    });
                });
                if self.photos.len() > 1 && ui.button("▦  Organizar em colagem").clicked() {
                    self.auto_arrange();
                }
                if self.selected.is_some() {
                    ui.label(
                        egui::RichText::new(
                            "Dica: Delete remove • botão direito abre o menu •\n\
                             duplo-clique recorta • Shift trava a proporção.",
                        )
                        .small()
                        .color(Color32::GRAY),
                    );
                }

                if self.crop_mode {
                    ui.separator();
                    ui.label(
                        egui::RichText::new("Modo recorte")
                            .strong()
                            .color(Color32::from_rgb(255, 200, 100)),
                    );
                    ui.label(
                        egui::RichText::new(
                            "Arraste as alças para definir o recorte;\n\
                             arraste dentro para reposicionar a imagem.",
                        )
                        .small(),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("✔  Concluir recorte").clicked() {
                            self.finish_crop();
                        }
                        if ui.button("↺  Recorte total").clicked() {
                            self.reset_crop();
                        }
                    });
                }

                ui.separator();
                ui.label(egui::RichText::new("Orientação da folha").strong());
                ui.horizontal(|ui| {
                    let mut changed = false;
                    changed |= ui.radio_value(&mut self.landscape, false, "Retrato").changed();
                    changed |= ui.radio_value(&mut self.landscape, true, "Paisagem").changed();
                    // Re-monta a colagem se houver fotos organizadas automaticamente
                    if changed && self.photos.iter().any(|p| p.cell.is_some()) {
                        self.auto_arrange();
                    }
                });
                ui.label(
                    egui::RichText::new("Muda sozinha conforme o arquivo adicionado.")
                        .small()
                        .color(Color32::GRAY),
                );

                ui.separator();
                ui.label(egui::RichText::new("Tamanho ao adicionar").strong());
                ui.horizontal(|ui| {
                    let mut changed = false;
                    changed |= ui
                        .radio_value(&mut self.real_size, true, "Tamanho real")
                        .on_hover_text(
                            "PDF no tamanho da página; foto pela resolução (DPI)\n\
                             gravada no arquivo (300 DPI se não informada).",
                        )
                        .changed();
                    changed |= ui
                        .radio_value(&mut self.real_size, false, "Caber na folha")
                        .on_hover_text("Preenche a área de impressão (colagem automática).")
                        .changed();
                    if changed {
                        self.apply_size_mode();
                    }
                });

                ui.separator();
                ui.label(egui::RichText::new("Bordas (mm)").strong());
                egui::Grid::new("margens").num_columns(2).show(ui, |ui| {
                    let labels = ["Esquerda", "Superior", "Direita", "Inferior"];
                    for (i, label) in labels.iter().enumerate() {
                        ui.label(*label);
                        ui.add(
                            egui::DragValue::new(&mut self.margins[i])
                                .speed(0.5)
                                .range(0.0..=80.0)
                                .suffix(" mm"),
                        );
                        ui.end_row();
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("0").on_hover_text("Sem bordas (folha toda)").clicked() {
                        self.margins = [0.0; 4];
                    }
                    if ui.button("5").clicked() {
                        self.margins = [5.0; 4];
                    }
                    if ui.button("10").clicked() {
                        self.margins = [10.0; 4];
                    }
                    ui.label(egui::RichText::new("rápido").small().color(Color32::GRAY));
                });

                ui.checkbox(&mut self.keep_aspect, "Manter proporção ao redimensionar (cantos)");

                ui.add_space(2.0);
                if ui
                    .checkbox(&mut self.black_frame, "Moldura preta entre as fotos")
                    .changed()
                {
                    self.reflow_cells();
                }
                if self.black_frame
                    && ui
                        .add(
                            egui::Slider::new(&mut self.frame_mm, 0.0..=15.0)
                                .text("espessura")
                                .suffix(" mm"),
                        )
                        .changed()
                {
                    self.reflow_cells();
                }

                if let Some(i) = self.selected {
                    if let Some(ph) = self.photos.get(i) {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "Selecionada: {:.1} × {:.1} cm",
                                ph.dest.w / 10.0,
                                ph.dest.h / 10.0
                            ))
                            .small(),
                        );
                        let cropped_w = ph.crop.width() * ph.img.width() as f32;
                        let dpi = cropped_w / (ph.dest.w / 25.4);
                        let quality = if dpi >= 200.0 {
                            "ótima"
                        } else if dpi >= 130.0 {
                            "boa"
                        } else {
                            "baixa"
                        };
                        ui.label(
                            egui::RichText::new(format!("Resolução: {dpi:.0} DPI ({quality})"))
                                .small(),
                        );
                    }
                }

                ui.separator();
                ui.add_space(4.0);
                let print_btn =
                    egui::Button::new(egui::RichText::new("🖨  Imprimir...").size(16.0))
                        .min_size(Vec2::new(210.0, 36.0));
                if ui.add_enabled(!self.photos.is_empty(), print_btn).clicked() {
                    self.start_print();
                }

                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(&self.status)
                        .small()
                        .color(Color32::LIGHT_GRAY),
                );

                if self.hevc_missing {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(
                            "Fotos HEIC de iPhone precisam do codec HEVC\n\
                             do Windows ('Extensões de Vídeo HEVC').",
                        )
                        .small()
                        .color(Color32::from_rgb(255, 200, 100)),
                    );
                    if ui.button("🛒  Instalar codec HEVC (Store)").clicked() {
                        let _ = std::process::Command::new("cmd")
                            .args([
                                "/C",
                                "start",
                                "",
                                "ms-windows-store://pdp/?ProductId=9NMZLZ57R3T7",
                            ])
                            .spawn();
                    }
                }

                // Rodapé: formatos aceitos
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(
                            "JPG, PNG, HEIC/HEIF (fotos de iPhone), AVIF,\n\
                             BMP, WEBP, TIFF, GIF e PDF",
                        )
                        .small()
                        .color(Color32::GRAY),
                    );
                    ui.label(egui::RichText::new("Formatos aceitos").small().strong());
                    ui.separator();
                });
            });
    }

    /// Tela principal de montagem (mover/redimensionar/selecionar fotos).
    fn canvas(&mut self, ui: &mut egui::Ui) {
        self.snap_x.clear();
        self.snap_y.clear();

        let (page_w, page_h) = self.page_size();
        let avail = ui.available_rect_before_wrap();
        let scale = ((avail.width() - 30.0) / page_w)
            .min((avail.height() - 30.0) / page_h)
            .max(0.1);
        let page_px = Vec2::new(page_w * scale, page_h * scale);
        let page_rect = Rect::from_center_size(avail.center(), page_px);
        self.view_origin = page_rect.min;
        self.view_scale = scale;

        let to_px = |mm_x: f32, mm_y: f32| -> Pos2 {
            Pos2::new(page_rect.min.x + mm_x * scale, page_rect.min.y + mm_y * scale)
        };
        let to_mm = |p: Pos2| -> Pos2 {
            Pos2::new(
                (p.x - page_rect.min.x) / scale,
                (p.y - page_rect.min.y) / scale,
            )
        };
        let rect_px =
            |r: RectMm| -> Rect { Rect::from_min_max(to_px(r.x, r.y), to_px(r.right(), r.bottom())) };

        let painter = ui.painter_at(avail);

        // Sombra + folha branca (o "quadro" A4)
        painter.rect_filled(
            page_rect.translate(Vec2::new(4.0, 4.0)),
            2.0,
            Color32::from_black_alpha(70),
        );
        painter.rect_filled(page_rect, 0.0, Color32::WHITE);
        painter.rect_stroke(
            page_rect,
            0.0,
            Stroke::new(1.0, Color32::from_gray(120)),
            egui::StrokeKind::Outside,
        );

        let area = self.print_area();
        let area_px = rect_px(area);
        let clip = area_px.intersect(page_rect);

        // Moldura preta atrás de cada foto (desenhada antes das imagens para
        // que nenhuma moldura cubra a foto vizinha)
        if self.black_frame && self.frame_mm > 0.0 {
            let clipped = painter.with_clip_rect(clip);
            for ph in &self.photos {
                clipped.rect_filled(rect_px(frame_of(ph.dest, self.frame_mm)), 0.0, Color32::BLACK);
            }
        }

        // Desenha todas as fotos (recortadas pela área de impressão)
        for (i, ph) in self.photos.iter().enumerate() {
            let dpx = rect_px(ph.dest);
            let clipped = painter.with_clip_rect(clip);
            clipped.image(ph.texture.id(), dpx, ph.crop, Color32::WHITE);
            let sel = self.selected == Some(i);
            painter.rect_stroke(
                dpx,
                0.0,
                Stroke::new(
                    if sel { 1.5 } else { 1.0 },
                    if sel {
                        Color32::from_rgb(70, 130, 240)
                    } else {
                        Color32::from_gray(150)
                    },
                ),
                egui::StrokeKind::Outside,
            );
        }

        // Linha tracejada da área de impressão (bordas)
        let dash = Stroke::new(1.0, Color32::from_rgb(150, 160, 180));
        for (a, b) in [
            (area_px.left_top(), area_px.right_top()),
            (area_px.right_top(), area_px.right_bottom()),
            (area_px.right_bottom(), area_px.left_bottom()),
            (area_px.left_bottom(), area_px.left_top()),
        ] {
            painter.extend(egui::Shape::dashed_line(&[a, b], dash, 6.0, 5.0));
        }

        // Pré-visualização ao arrastar arquivos do Explorer sobre a janela
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        if hovering {
            let pos = ui.ctx().input(|i| i.pointer.hover_pos().or_else(|| i.pointer.latest_pos()));
            let target = pos.and_then(|p| {
                (0..self.photos.len())
                    .rev()
                    .find(|&i| rect_px(self.photos[i].dest).contains(p))
            });
            match target {
                Some(i) => {
                    let dpx = rect_px(self.photos[i].dest);
                    painter.rect_filled(dpx, 0.0, Color32::from_rgba_unmultiplied(0, 150, 80, 70));
                    painter.rect_stroke(
                        dpx,
                        0.0,
                        Stroke::new(2.5, Color32::from_rgb(0, 180, 90)),
                        egui::StrokeKind::Outside,
                    );
                    painter.text(
                        dpx.center(),
                        egui::Align2::CENTER_CENTER,
                        "Substituir esta foto",
                        egui::FontId::proportional(16.0),
                        Color32::WHITE,
                    );
                }
                None => {
                    painter.rect_stroke(
                        page_rect,
                        0.0,
                        Stroke::new(2.5, Color32::from_rgb(0, 150, 255)),
                        egui::StrokeKind::Outside,
                    );
                    painter.text(
                        Pos2::new(page_rect.center().x, page_rect.top() + 18.0),
                        egui::Align2::CENTER_CENTER,
                        "Soltar para adicionar à página",
                        egui::FontId::proportional(16.0),
                        Color32::from_rgb(0, 150, 255),
                    );
                }
            }
        }

        if self.photos.is_empty() {
            painter.text(
                page_rect.center(),
                egui::Align2::CENTER_CENTER,
                "Adicione ou arraste fotos aqui",
                egui::FontId::proportional(18.0),
                Color32::from_gray(150),
            );
            return;
        }

        let response = ui.interact(avail, ui.id().with("canvas"), Sense::click_and_drag());
        let thr = 12.0 / scale;
        let len = self.photos.len();

        // Cursor conforme a posição (sobre a foto selecionada)
        if self.drag.is_none() {
            if let (Some(i), Some(p)) = (self.selected, response.hover_pos()) {
                if let Some(ph) = self.photos.get(i) {
                    let dpx = rect_px(ph.dest);
                    if let Some(h) = hit_handle(dpx, p) {
                        ui.output_mut(|o| o.cursor_icon = h.cursor());
                    } else if dpx.contains(p) {
                        ui.output_mut(|o| o.cursor_icon = CursorIcon::Grab);
                    }
                }
            }
        }

        if response.drag_started() {
            if let Some(p) = response.interact_pointer_pos() {
                let pm = to_mm(p);
                let mut started = false;
                // 1) alça da foto já selecionada
                if let Some(i) = self.selected {
                    if let Some(ph) = self.photos.get(i) {
                        if let Some(h) = hit_handle(rect_px(ph.dest), p) {
                            self.drag = Some(DragMode::Resize(h));
                            started = true;
                        }
                    }
                }
                // 2) senão, escolhe a foto mais ao topo sob o cursor
                if !started {
                    for i in (0..len).rev() {
                        let dpx = rect_px(self.photos[i].dest);
                        if let Some(h) = hit_handle(dpx, p) {
                            self.selected = Some(i);
                            self.drag = Some(DragMode::Resize(h));
                            started = true;
                            break;
                        }
                        if dpx.contains(p) {
                            self.selected = Some(i);
                            self.drag = Some(DragMode::Move {
                                grab: Vec2::new(pm.x - self.photos[i].dest.x, pm.y - self.photos[i].dest.y),
                            });
                            started = true;
                            break;
                        }
                    }
                }
                if !started {
                    self.drag = None;
                } else if let Some(i) = self.selected {
                    // Movida manualmente: deixa de pertencer à colagem
                    if let Some(ph) = self.photos.get_mut(i) {
                        ph.cell = None;
                    }
                }
            }
        }

        let frame_m = if self.black_frame { self.frame_mm } else { 0.0 };
        let shift = ui.input(|i| i.modifiers.shift);

        if response.dragged() {
            if let (Some(mode), Some(i), Some(p)) =
                (self.drag, self.selected, response.interact_pointer_pos())
            {
                let pm = to_mm(p);
                let r = self.photos[i].dest;
                // Shift mantém a proporção ao arrastar os cantos (mesmo sem a opção)
                let keep = if self.keep_aspect || shift {
                    Some(self.photos[i].crop_aspect())
                } else {
                    None
                };
                let nr = match mode {
                    DragMode::Move { grab } => {
                        ui.output_mut(|o| o.cursor_icon = CursorIcon::Grabbing);
                        self.move_rect(r, pm, grab, thr, frame_m)
                    }
                    DragMode::Resize(h) => {
                        ui.output_mut(|o| o.cursor_icon = h.cursor());
                        self.resize_rect(r, keep, h, pm, thr, frame_m)
                    }
                };
                self.photos[i].dest = nr;
            }
        }

        if response.drag_stopped() {
            self.drag = None;
        }

        // Clique simples: seleciona a foto sob o cursor (ou nada)
        if response.clicked() {
            if let Some(p) = response.interact_pointer_pos() {
                let mut hit = None;
                for i in (0..len).rev() {
                    if rect_px(self.photos[i].dest).contains(p) {
                        hit = Some(i);
                        break;
                    }
                }
                self.selected = hit;
            }
        }

        // Duplo-clique sobre uma foto abre o recorte
        if response.double_clicked() {
            if let Some(p) = response.interact_pointer_pos() {
                for i in (0..len).rev() {
                    if rect_px(self.photos[i].dest).contains(p) {
                        self.selected = Some(i);
                        self.crop_mode = true;
                        break;
                    }
                }
            }
        }

        // Botão direito: seleciona a foto sob o cursor e abre o menu de contexto
        if response.secondary_clicked() {
            if let Some(p) = response.interact_pointer_pos() {
                let mut hit = None;
                for i in (0..len).rev() {
                    if rect_px(self.photos[i].dest).contains(p) {
                        hit = Some(i);
                        break;
                    }
                }
                self.selected = hit;
            }
        }
        response.context_menu(|ui| {
            if self.selected.is_some() {
                if ui.button("✂  Recortar").clicked() {
                    self.crop_mode = true;
                    ui.close_menu();
                }
                if ui.button("⬆  Trazer para frente").clicked() {
                    self.bring_to_front();
                    ui.close_menu();
                }
                if ui.button("🎯  Centralizar").clicked() {
                    self.center_selected();
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("🗑  Excluir foto").clicked() {
                    self.remove_selected();
                    ui.close_menu();
                }
            } else {
                ui.label("Clique com o direito sobre uma foto.");
            }
        });

        // Linhas-guia azuis quando ancorado
        let guide = Stroke::new(1.5, Color32::from_rgb(0, 150, 255));
        for &x in &self.snap_x {
            let p = to_px(x, 0.0);
            painter.line_segment(
                [
                    Pos2::new(p.x, page_rect.top()),
                    Pos2::new(p.x, page_rect.bottom()),
                ],
                guide,
            );
        }
        for &y in &self.snap_y {
            let p = to_px(0.0, y);
            painter.line_segment(
                [
                    Pos2::new(page_rect.left(), p.y),
                    Pos2::new(page_rect.right(), p.y),
                ],
                guide,
            );
        }

        // Alças da foto selecionada
        if let Some(i) = self.selected {
            if let Some(ph) = self.photos.get(i) {
                for c in handle_points(rect_px(ph.dest)) {
                    let r = Rect::from_center_size(c, Vec2::splat(9.0));
                    painter.rect_filled(r, 2.0, Color32::WHITE);
                    painter.rect_stroke(
                        r,
                        2.0,
                        Stroke::new(1.5, Color32::from_rgb(70, 130, 240)),
                        egui::StrokeKind::Outside,
                    );
                }
            }
        }
    }

    /// Editor de recorte da foto selecionada.
    fn canvas_crop(&mut self, ui: &mut egui::Ui) {
        let Some(idx) = self.selected else {
            self.crop_mode = false;
            return;
        };
        let (tex_id, img_aspect) = {
            let ph = &self.photos[idx];
            (ph.texture.id(), ph.img_aspect)
        };

        let avail = ui.available_rect_before_wrap();
        let maxw = (avail.width() - 40.0).max(10.0);
        let maxh = (avail.height() - 60.0).max(10.0);
        let (dw, dh) = if img_aspect >= maxw / maxh {
            (maxw, maxw / img_aspect)
        } else {
            (maxh * img_aspect, maxh)
        };
        let img_rect = Rect::from_center_size(
            Pos2::new(avail.center().x, avail.center().y + 10.0),
            Vec2::new(dw, dh),
        );

        let painter = ui.painter_at(avail);
        painter.text(
            Pos2::new(avail.center().x, avail.top() + 16.0),
            egui::Align2::CENTER_CENTER,
            "Recorte: arraste as alças ou arraste dentro da seleção",
            egui::FontId::proportional(15.0),
            Color32::from_gray(210),
        );

        let full = full_crop();
        // Imagem inteira escurecida ao fundo
        painter.image(tex_id, img_rect, full, Color32::from_gray(90));

        let crop = self.photos[idx].crop;
        let crop_px = Rect::from_min_max(
            Pos2::new(
                img_rect.left() + crop.min.x * img_rect.width(),
                img_rect.top() + crop.min.y * img_rect.height(),
            ),
            Pos2::new(
                img_rect.left() + crop.max.x * img_rect.width(),
                img_rect.top() + crop.max.y * img_rect.height(),
            ),
        );
        // Região recortada em destaque
        painter.image(tex_id, crop_px, crop, Color32::WHITE);
        painter.rect_stroke(
            crop_px,
            0.0,
            Stroke::new(1.5, Color32::from_rgb(70, 130, 240)),
            egui::StrokeKind::Outside,
        );

        let response = ui.interact(avail, ui.id().with("cropcanvas"), Sense::click_and_drag());
        let to_uv = |p: Pos2| -> Pos2 {
            Pos2::new(
                ((p.x - img_rect.left()) / img_rect.width()).clamp(0.0, 1.0),
                ((p.y - img_rect.top()) / img_rect.height()).clamp(0.0, 1.0),
            )
        };

        if self.crop_drag.is_none() {
            if let Some(p) = response.hover_pos() {
                if let Some(h) = hit_handle(crop_px, p) {
                    ui.output_mut(|o| o.cursor_icon = h.cursor());
                } else if crop_px.contains(p) {
                    ui.output_mut(|o| o.cursor_icon = CursorIcon::Grab);
                }
            }
        }

        if response.drag_started() {
            if let Some(p) = response.interact_pointer_pos() {
                if let Some(h) = hit_handle(crop_px, p) {
                    self.crop_drag = Some(DragMode::Resize(h));
                } else if crop_px.contains(p) {
                    let uv = to_uv(p);
                    self.crop_drag = Some(DragMode::Move {
                        grab: Vec2::new(uv.x - crop.min.x, uv.y - crop.min.y),
                    });
                } else {
                    self.crop_drag = None;
                }
            }
        }

        if response.dragged() {
            if let (Some(mode), Some(p)) = (self.crop_drag, response.interact_pointer_pos()) {
                let uv = to_uv(p);
                let mut c = self.photos[idx].crop;
                match mode {
                    DragMode::Move { grab } => {
                        ui.output_mut(|o| o.cursor_icon = CursorIcon::Grabbing);
                        let (w, h) = (c.width(), c.height());
                        let nx = (uv.x - grab.x).clamp(0.0, 1.0 - w);
                        let ny = (uv.y - grab.y).clamp(0.0, 1.0 - h);
                        c = Rect::from_min_size(Pos2::new(nx, ny), Vec2::new(w, h));
                    }
                    DragMode::Resize(h) => {
                        ui.output_mut(|o| o.cursor_icon = h.cursor());
                        let (mut l, mut r, mut t, mut b) = (c.min.x, c.max.x, c.min.y, c.max.y);
                        if h.moves_left() {
                            l = uv.x.min(r - MIN_CROP);
                        }
                        if h.moves_right() {
                            r = uv.x.max(l + MIN_CROP);
                        }
                        if h.moves_top() {
                            t = uv.y.min(b - MIN_CROP);
                        }
                        if h.moves_bottom() {
                            b = uv.y.max(t + MIN_CROP);
                        }
                        c = Rect::from_min_max(Pos2::new(l, t), Pos2::new(r, b));
                    }
                }
                self.photos[idx].crop = c;
            }
        }

        if response.drag_stopped() {
            self.crop_drag = None;
        }

        // Alças do recorte
        for c in handle_points(crop_px) {
            let r = Rect::from_center_size(c, Vec2::splat(9.0));
            painter.rect_filled(r, 2.0, Color32::WHITE);
            painter.rect_stroke(
                r,
                2.0,
                Stroke::new(1.5, Color32::from_rgb(70, 130, 240)),
                egui::StrokeKind::Outside,
            );
        }
    }
}

fn fit_rect(area: RectMm, aspect: f32) -> RectMm {
    let area_aspect = area.w / area.h;
    let (w, h) = if aspect >= area_aspect {
        (area.w, area.w / aspect)
    } else {
        (area.h * aspect, area.h)
    };
    RectMm {
        x: area.x + (area.w - w) / 2.0,
        y: area.y + (area.h - h) / 2.0,
        w,
        h,
    }
}

fn full_crop() -> Rect {
    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0))
}

/// Retângulo da moldura: o destino da foto expandido por `m` mm em cada lado.
fn frame_of(dest: RectMm, m: f32) -> RectMm {
    RectMm {
        x: dest.x - m,
        y: dest.y - m,
        w: dest.w + 2.0 * m,
        h: dest.h + 2.0 * m,
    }
}

/// Células de uma grade simples (preenchidas linha a linha).
fn grid_cells(area: RectMm, cols: usize, rows: usize, n: usize) -> Vec<RectMm> {
    let cw = area.w / cols as f32;
    let ch = area.h / rows as f32;
    (0..n)
        .map(|i| RectMm {
            x: area.x + (i % cols) as f32 * cw,
            y: area.y + (i / cols) as f32 * ch,
            w: cw,
            h: ch,
        })
        .collect()
}

/// Modelos de colagem por quantidade de fotos (uma grande + menores), adaptados
/// à orientação da área. Sempre devolve exatamente `n` células cobrindo a área.
fn template_cells(n: usize, area: RectMm) -> Vec<RectMm> {
    let portrait = area.h >= area.w;
    let mk = |fx: f32, fy: f32, fw: f32, fh: f32| RectMm {
        x: area.x + fx * area.w,
        y: area.y + fy * area.h,
        w: fw * area.w,
        h: fh * area.h,
    };
    match n {
        0 => vec![],
        1 => vec![mk(0.0, 0.0, 1.0, 1.0)],
        2 => {
            if portrait {
                vec![mk(0.0, 0.0, 1.0, 0.5), mk(0.0, 0.5, 1.0, 0.5)]
            } else {
                vec![mk(0.0, 0.0, 0.5, 1.0), mk(0.5, 0.0, 0.5, 1.0)]
            }
        }
        3 => {
            if portrait {
                vec![
                    mk(0.0, 0.0, 1.0, 0.6),
                    mk(0.0, 0.6, 0.5, 0.4),
                    mk(0.5, 0.6, 0.5, 0.4),
                ]
            } else {
                vec![
                    mk(0.0, 0.0, 0.6, 1.0),
                    mk(0.6, 0.0, 0.4, 0.5),
                    mk(0.6, 0.5, 0.4, 0.5),
                ]
            }
        }
        4 => {
            // Uma foto grande + 3 menores ao lado (exemplo pedido)
            if portrait {
                let mut v = vec![mk(0.0, 0.0, 1.0, 0.62)];
                for i in 0..3 {
                    v.push(mk(i as f32 / 3.0, 0.62, 1.0 / 3.0, 0.38));
                }
                v
            } else {
                let mut v = vec![mk(0.0, 0.0, 0.62, 1.0)];
                for i in 0..3 {
                    v.push(mk(0.62, i as f32 / 3.0, 0.38, 1.0 / 3.0));
                }
                v
            }
        }
        5 => {
            if portrait {
                let mut v = vec![mk(0.0, 0.0, 1.0, 0.6)];
                for i in 0..4 {
                    v.push(mk(i as f32 / 4.0, 0.6, 0.25, 0.4));
                }
                v
            } else {
                let mut v = vec![mk(0.0, 0.0, 0.6, 1.0)];
                for i in 0..4 {
                    v.push(mk(0.6, i as f32 / 4.0, 0.4, 0.25));
                }
                v
            }
        }
        6 => {
            if portrait {
                grid_cells(area, 2, 3, 6)
            } else {
                grid_cells(area, 3, 2, 6)
            }
        }
        _ => {
            let cols = (n as f32).sqrt().ceil() as usize;
            let rows = n.div_ceil(cols);
            grid_cells(area, cols, rows, n)
        }
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "imagem".into())
}

/// Cria a textura de pré-visualização (reduzida se a imagem for muito grande;
/// a impressão usa sempre a resolução original).
fn make_texture(ctx: &egui::Context, name: &str, rgba: &RgbaImage) -> TextureHandle {
    let preview = if rgba.width() > PREVIEW_MAX_PX || rgba.height() > PREVIEW_MAX_PX {
        let s = PREVIEW_MAX_PX as f32 / rgba.width().max(rgba.height()) as f32;
        image::imageops::resize(
            rgba,
            ((rgba.width() as f32 * s) as u32).max(1),
            ((rgba.height() as f32 * s) as u32).max(1),
            image::imageops::FilterType::Triangle,
        )
    } else {
        rgba.clone()
    };
    let size = [preview.width() as usize, preview.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, preview.as_raw());
    ctx.load_texture(name, color, egui::TextureOptions::LINEAR)
}

fn handle_points(r: Rect) -> [Pos2; 8] {
    [
        r.left_top(),
        r.center_top(),
        r.right_top(),
        r.right_center(),
        r.right_bottom(),
        r.center_bottom(),
        r.left_bottom(),
        r.left_center(),
    ]
}

fn hit_handle(photo_px: Rect, p: Pos2) -> Option<Handle> {
    const R: f32 = 12.0;
    let pts = [
        (Handle::NW, photo_px.left_top()),
        (Handle::NE, photo_px.right_top()),
        (Handle::SW, photo_px.left_bottom()),
        (Handle::SE, photo_px.right_bottom()),
        (Handle::N, photo_px.center_top()),
        (Handle::S, photo_px.center_bottom()),
        (Handle::W, photo_px.left_center()),
        (Handle::E, photo_px.right_center()),
    ];
    pts.iter().find(|(_, c)| c.distance(p) <= R).map(|(h, _)| *h)
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Arquivos arrastados do Explorer: soltar sobre uma foto substitui aquela
        // foto; soltar numa área vazia da folha adiciona uma nova.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            let drop_pos = ctx.input(|i| i.pointer.hover_pos().or_else(|| i.pointer.latest_pos()));
            let target = self.photo_at_screen(drop_pos);
            for (k, p) in dropped.into_iter().enumerate() {
                match target {
                    Some(t) if k == 0 => self.replace_photo(ctx, t, p),
                    _ => self.load_image(ctx, p),
                }
            }
        }

        // Esc sai do modo recorte
        if self.crop_mode && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.finish_crop();
        }

        // Delete/Backspace remove a foto selecionada (exceto editando um campo)
        if !self.crop_mode
            && self.selected.is_some()
            && ctx.memory(|m| m.focused().is_none())
            && ctx.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        {
            self.remove_selected();
        }

        // Resultado da impressão (thread separada)
        if let Some(rx) = &self.print_rx {
            if let Ok(msg) = rx.try_recv() {
                self.status = msg;
                self.print_rx = None;
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }

        self.side_panel(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(Color32::from_gray(55)))
            .show(ctx, |ui| {
                if self.crop_mode && self.selected.is_some() {
                    self.canvas_crop(ui);
                } else {
                    self.crop_mode = false;
                    self.canvas(ui);
                }
            });
    }
}

// ---------------------------------------------------------------------------
// Tamanho físico (DPI) das imagens
// ---------------------------------------------------------------------------

/// DPI assumido quando o arquivo não informa (ou informa o padrão de tela).
const DEFAULT_DPI: f32 = 300.0;

/// Tamanho físico (mm) de uma imagem, pelo DPI gravado no arquivo (JPEG JFIF ou
/// PNG pHYs). 72/96 DPI são o padrão de tela gravado por câmeras e celulares e
/// não representam um tamanho de impressão — nesse caso assume `DEFAULT_DPI`.
fn image_phys_mm(path: &std::path::Path, w: u32, h: u32) -> (f32, f32) {
    let (dx, dy) = read_dpi(path)
        .filter(|&(x, y)| x > 0.0 && y > 0.0 && !is_screen_dpi(x) && !is_screen_dpi(y))
        .unwrap_or((DEFAULT_DPI, DEFAULT_DPI));
    (w as f32 * 25.4 / dx, h as f32 * 25.4 / dy)
}

fn is_screen_dpi(d: f32) -> bool {
    (d - 72.0).abs() < 0.5 || (d - 96.0).abs() < 0.5
}

/// Lê a densidade (DPI x, y) de JPEG (APP0/JFIF) ou PNG (pHYs). None se ausente.
fn read_dpi(path: &std::path::Path) -> Option<(f32, f32)> {
    let data = std::fs::read(path).ok()?;
    if data.starts_with(b"\x89PNG") {
        return png_dpi(&data);
    }
    if data.starts_with(&[0xFF, 0xD8]) {
        return jpeg_dpi(&data);
    }
    None
}

fn png_dpi(data: &[u8]) -> Option<(f32, f32)> {
    let be32 = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    let mut pos = 8;
    while pos + 8 <= data.len() {
        let len = be32(&data[pos..pos + 4]) as usize;
        let kind = &data[pos + 4..pos + 8];
        let body = data.get(pos + 8..pos + 8 + len)?;
        if kind == b"pHYs" && len >= 9 {
            if body[8] == 1 {
                let x = be32(&body[0..4]) as f32 * 0.0254;
                let y = be32(&body[4..8]) as f32 * 0.0254;
                return Some((x, y));
            }
            return None;
        }
        if kind == b"IDAT" || kind == b"IEND" {
            return None;
        }
        pos += 12 + len;
    }
    None
}

fn jpeg_dpi(data: &[u8]) -> Option<(f32, f32)> {
    let mut pos = 2;
    while pos + 4 <= data.len() {
        if data[pos] != 0xFF {
            return None;
        }
        let marker = data[pos + 1];
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            pos += 2;
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            return None; // início dos dados / fim: sem APP0
        }
        let len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        let seg = data.get(pos + 4..pos + 2 + len)?;
        if marker == 0xE0 && seg.len() >= 12 && &seg[0..5] == b"JFIF\0" {
            let units = seg[7];
            let x = u16::from_be_bytes([seg[8], seg[9]]) as f32;
            let y = u16::from_be_bytes([seg[10], seg[11]]) as f32;
            return match units {
                1 => Some((x, y)),
                2 => Some((x * 2.54, y * 2.54)),
                _ => None,
            };
        }
        pos += 2 + len;
    }
    None
}

// ---------------------------------------------------------------------------
// Decodificação de imagens (HEIC/HEIF/AVIF via WIC, demais via crate `image`)
// ---------------------------------------------------------------------------

fn decode_image(path: &std::path::Path) -> Result<RgbaImage, String> {
    #[cfg(windows)]
    {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        // PDF: renderiza a 1ª página com o motor nativo do Windows (Windows.Data.Pdf)
        if ext == "pdf" {
            return decode_pdf(path);
        }
        // HEIC/HEIF: decodificador libheif embutido (não depende de codec do Windows);
        // WIC fica como reserva
        if matches!(ext.as_str(), "heic" | "heif" | "hif") {
            return match decode_with_libheif(path) {
                Ok(img) => Ok(img),
                Err(e) => decode_with_wic(path).map_err(|_| e),
            };
        }
        if ext == "avif" {
            return match decode_with_wic(path) {
                Ok(img) => Ok(img),
                Err(e) => decode_with_libheif(path).map_err(|_| e),
            };
        }
    }

    match image::open(path) {
        Ok(i) => Ok(i.to_rgba8()),
        Err(e) => {
            // Últimas tentativas: decodificadores do Windows e libheif
            #[cfg(windows)]
            {
                if let Ok(img) = decode_with_wic(path) {
                    return Ok(img);
                }
                if let Ok(img) = decode_with_libheif(path) {
                    return Ok(img);
                }
            }
            Err(e.to_string())
        }
    }
}

/// Renderiza a página `page` (índice base 0) de um PDF usando o motor nativo do
/// Windows (Windows.Data.Pdf — o mesmo do Edge) a ~200 DPI. Devolve a imagem, o
/// número total de páginas e o tamanho físico da página em mm.
#[cfg(windows)]
fn render_pdf_page(
    path: &std::path::Path,
    page: u32,
) -> Result<(RgbaImage, u32, (f32, f32)), String> {
    use windows::core::HSTRING;
    use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
    use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
    use windows::Storage::StorageFile;

    // Caminho absoluto (a API do Windows exige caminho completo, sem o prefixo \\?\)
    let abs = std::fs::canonicalize(path).map_err(|e| format!("PDF: {e}"))?;
    let abs_str = abs.to_string_lossy().replace(r"\\?\", "");
    let h = HSTRING::from(abs_str.as_str());

    let file = StorageFile::GetFileFromPathAsync(&h)
        .map_err(|e| format!("PDF: {e}"))?
        .get()
        .map_err(|e| format!("PDF: não foi possível abrir o arquivo ({e})"))?;
    let doc = PdfDocument::LoadFromFileAsync(&file)
        .map_err(|e| format!("PDF: {e}"))?
        .get()
        .map_err(|e| format!("PDF: arquivo inválido ou protegido por senha ({e})"))?;

    let count = doc.PageCount().map_err(|e| format!("PDF: {e}"))?;
    if count == 0 {
        return Err("PDF: documento sem páginas".into());
    }
    let idx = page.min(count - 1);
    let page = doc.GetPage(idx).map_err(|e| format!("PDF: {e}"))?;
    let size = page.Size().map_err(|e| format!("PDF: {e}"))?;

    // Tamanho da página vem em DIPs (96 DPI); renderiza a ~200 DPI
    let phys = (size.Width * 25.4 / 96.0, size.Height * 25.4 / 96.0);
    let dest_w = ((size.Width * 200.0 / 96.0) as u32).clamp(1, 5000);

    let stream = InMemoryRandomAccessStream::new().map_err(|e| format!("PDF: {e}"))?;
    let opts = PdfPageRenderOptions::new().map_err(|e| format!("PDF: {e}"))?;
    opts.SetDestinationWidth(dest_w)
        .map_err(|e| format!("PDF: {e}"))?;
    page.RenderWithOptionsToStreamAsync(&stream, &opts)
        .map_err(|e| format!("PDF: {e}"))?
        .get()
        .map_err(|e| format!("PDF: falha ao renderizar a página ({e})"))?;

    let n = stream.Size().map_err(|e| format!("PDF: {e}"))? as u32;
    let input = stream.GetInputStreamAt(0).map_err(|e| format!("PDF: {e}"))?;
    let reader = DataReader::CreateDataReader(&input).map_err(|e| format!("PDF: {e}"))?;
    reader
        .LoadAsync(n)
        .map_err(|e| format!("PDF: {e}"))?
        .get()
        .map_err(|e| format!("PDF: {e}"))?;
    let mut buf = vec![0u8; n as usize];
    reader.ReadBytes(&mut buf).map_err(|e| format!("PDF: {e}"))?;

    let img = image::load_from_memory(&buf)
        .map(|i| i.to_rgba8())
        .map_err(|e| format!("PDF: falha ao decodificar a página renderizada ({e})"))?;
    Ok((img, count, phys))
}

#[cfg(windows)]
fn decode_pdf(path: &std::path::Path) -> Result<RgbaImage, String> {
    render_pdf_page(path, 0).map(|(img, _, _)| img)
}

#[cfg(not(windows))]
fn render_pdf_page(_: &std::path::Path, _: u32) -> Result<(RgbaImage, u32, (f32, f32)), String> {
    Err("abertura de PDF disponível apenas no Windows".into())
}

#[cfg(not(windows))]
fn decode_pdf(_: &std::path::Path) -> Result<RgbaImage, String> {
    Err("abertura de PDF disponível apenas no Windows".into())
}

/// Decodifica HEIC/HEIF com o libheif embutido no executável (libde265 para HEVC).
#[cfg(windows)]
fn decode_with_libheif(path: &std::path::Path) -> Result<RgbaImage, String> {
    use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};

    let lib_heif = LibHeif::new();
    let ctx = HeifContext::read_from_file(
        path.to_str().ok_or("caminho com caracteres inválidos")?,
    )
    .map_err(|e| format!("HEIF: {e}"))?;
    let handle = ctx.primary_image_handle().map_err(|e| format!("HEIF: {e}"))?;
    let img = lib_heif
        .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgba), None)
        .map_err(|e| format!("HEIF: {e}"))?;
    let planes = img.planes();
    let inter = planes
        .interleaved
        .ok_or("HEIF: imagem sem plano de cores intercalado")?;
    let (w, h) = (inter.width, inter.height);
    let row_bytes = w as usize * 4;
    let mut buf = Vec::with_capacity(row_bytes * h as usize);
    for row in 0..h as usize {
        let start = row * inter.stride;
        buf.extend_from_slice(&inter.data[start..start + row_bytes]);
    }
    RgbaImage::from_raw(w, h, buf).ok_or_else(|| "HEIF: buffer de imagem inválido".into())
}

/// Decodifica usando o Windows Imaging Component (mesmo decodificador do app
/// Fotos), que cobre HEIC/HEIF quando as extensões do sistema estão presentes.
#[cfg(windows)]
fn decode_with_wic(path: &std::path::Path) -> Result<RgbaImage, String> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::{null, null_mut};
    use winapi::shared::wtypesbase::CLSCTX_INPROC_SERVER;
    use winapi::um::combaseapi::{CoCreateInstance, CoInitializeEx};
    use winapi::um::objbase::COINIT_APARTMENTTHREADED;
    use winapi::um::unknwnbase::IUnknown;
    use winapi::um::wincodec::{
        CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICBitmapDecoder,
        IWICBitmapFrameDecode, IWICBitmapSource, IWICFormatConverter, IWICImagingFactory,
        WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
    };
    use winapi::um::winnt::GENERIC_READ;
    use winapi::Interface;

    unsafe fn rel<T>(p: *mut T) {
        if !p.is_null() {
            unsafe { (*(p as *mut IUnknown)).Release() };
        }
    }

    unsafe {
        // Pode já ter sido inicializado pelo runtime da janela; o retorno não importa
        CoInitializeEx(null_mut(), COINIT_APARTMENTTHREADED);

        let mut factory: *mut IWICImagingFactory = null_mut();
        let mut decoder: *mut IWICBitmapDecoder = null_mut();
        let mut frame: *mut IWICBitmapFrameDecode = null_mut();
        let mut conv: *mut IWICFormatConverter = null_mut();

        let result = (|| -> Result<RgbaImage, String> {
            let hr = CoCreateInstance(
                &CLSID_WICImagingFactory,
                null_mut(),
                CLSCTX_INPROC_SERVER,
                &IWICImagingFactory::uuidof(),
                &mut factory as *mut _ as *mut _,
            );
            if hr < 0 || factory.is_null() {
                return Err("WIC indisponível neste sistema".into());
            }

            let wide: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let hr = (*factory).CreateDecoderFromFilename(
                wide.as_ptr(),
                null(),
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
                &mut decoder,
            );
            if hr < 0 || decoder.is_null() {
                return Err(format!(
                    "o Windows não tem decodificador para este arquivo (HRESULT 0x{:08X})",
                    hr as u32
                ));
            }

            let hr = (*decoder).GetFrame(0, &mut frame);
            if hr < 0 || frame.is_null() {
                return Err("falha ao ler o quadro da imagem".into());
            }

            let hr = (*factory).CreateFormatConverter(&mut conv);
            if hr < 0 || conv.is_null() {
                return Err("falha ao criar conversor de formato".into());
            }
            let hr = (*conv).Initialize(
                frame as *mut IWICBitmapSource,
                &GUID_WICPixelFormat32bppRGBA,
                WICBitmapDitherTypeNone,
                null_mut(),
                0.0,
                WICBitmapPaletteTypeCustom,
            );
            if hr < 0 {
                return Err("falha ao converter a imagem para RGBA".into());
            }

            let (mut w, mut h) = (0u32, 0u32);
            (*conv).GetSize(&mut w, &mut h);
            if w == 0 || h == 0 {
                return Err("imagem com dimensões inválidas".into());
            }

            let stride = w as usize * 4;
            let mut buf = vec![0u8; stride * h as usize];
            let hr = (*conv).CopyPixels(null(), stride as u32, buf.len() as u32, buf.as_mut_ptr());
            if hr < 0 {
                // MF_E_TOPO_CODEC_NOT_FOUND: contêiner HEIF aberto, mas sem codec HEVC
                if hr as u32 == 0xC00D5212 {
                    return Err("o codec HEVC não está instalado".into());
                }
                return Err(format!(
                    "falha ao decodificar os pixels da imagem (HRESULT 0x{:08X})",
                    hr as u32
                ));
            }

            RgbaImage::from_raw(w, h, buf).ok_or_else(|| "buffer de imagem inválido".into())
        })();

        rel(conv);
        rel(frame);
        rel(decoder);
        rel(factory);
        result
    }
}

// ---------------------------------------------------------------------------
// Impressão via GDI (Windows) — várias fotos na mesma página
// ---------------------------------------------------------------------------

/// Janela principal do aplicativo (dona do diálogo de impressão), como valor
/// inteiro para poder ir para a thread de impressão.
#[cfg(windows)]
fn owner_window() -> isize {
    use winapi::um::winuser::{FindWindowW, GetForegroundWindow};
    unsafe {
        let h = GetForegroundWindow();
        if !h.is_null() {
            return h as isize;
        }
        let title: Vec<u16> = "Impressão de Foto A4"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        FindWindowW(std::ptr::null(), title.as_ptr()) as isize
    }
}

#[cfg(not(windows))]
fn owner_window() -> isize {
    0
}

#[cfg(windows)]
fn print_photos(
    items: &[(Arc<RgbaImage>, [f32; 4], RectMm)],
    area: RectMm,
    landscape: bool,
    frame_mm: f32,
    owner: isize,
) -> Result<bool, String> {
    use std::mem::{size_of, zeroed};
    use winapi::shared::windef::{HBRUSH, HWND, RECT};
    use winapi::um::combaseapi::CoInitializeEx;
    use winapi::um::commdlg::{
        PrintDlgExW, PD_NOCURRENTPAGE, PD_NOPAGENUMS, PD_NOSELECTION, PD_RESULT_PRINT,
        PD_RETURNDC, PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGEXW, START_PAGE_GENERAL,
    };
    use winapi::um::objbase::COINIT_APARTMENTTHREADED;
    use winapi::um::winbase::{GlobalFree, GlobalLock, GlobalUnlock};
    use winapi::um::winuser::FillRect;
    use winapi::um::wingdi::{
        DeleteDC, EndDoc, EndPage, GetDeviceCaps, GetStockObject, IntersectClipRect, ResetDCW,
        SetBrushOrgEx, SetStretchBltMode, StartDocW, StartPage, StretchDIBits, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, BLACK_BRUSH, DEVMODEW, DIB_RGB_COLORS, DMORIENT_LANDSCAPE,
        DMORIENT_PORTRAIT, DMPAPER_A4, DM_ORIENTATION, DM_PAPERSIZE, DOCINFOW, HALFTONE, LOGPIXELSX,
        LOGPIXELSY, PHYSICALOFFSETX, PHYSICALOFFSETY, SRCCOPY,
    };

    unsafe {
        // Diálogo de impressão moderno (PrintDlgEx, com aba "Geral"); é baseado
        // em COM e exige uma janela dona válida.
        CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED);
        let mut pd: PRINTDLGEXW = zeroed();
        pd.lStructSize = size_of::<PRINTDLGEXW>() as u32;
        pd.hwndOwner = owner as HWND;
        pd.Flags = PD_RETURNDC
            | PD_NOPAGENUMS
            | PD_NOSELECTION
            | PD_NOCURRENTPAGE
            | PD_USEDEVMODECOPIESANDCOLLATE;
        pd.nCopies = 1;
        pd.nStartPage = START_PAGE_GENERAL;
        let hr = PrintDlgExW(&mut pd);
        if hr < 0 {
            return Err(format!(
                "falha ao abrir o diálogo de impressão (HRESULT 0x{:08X})",
                hr as u32
            ));
        }
        if pd.dwResultAction != PD_RESULT_PRINT {
            if !pd.hDC.is_null() {
                DeleteDC(pd.hDC);
            }
            if !pd.hDevMode.is_null() {
                GlobalFree(pd.hDevMode);
            }
            if !pd.hDevNames.is_null() {
                GlobalFree(pd.hDevNames);
            }
            return Ok(false); // usuário cancelou (ou só aplicou preferências)
        }
        let hdc = pd.hDC;
        if hdc.is_null() {
            return Err("não foi possível obter o contexto da impressora".into());
        }

        // Força papel A4 e a orientação escolhida no aplicativo
        if !pd.hDevMode.is_null() {
            let dm = GlobalLock(pd.hDevMode) as *mut DEVMODEW;
            if !dm.is_null() {
                (*dm).dmFields |= DM_ORIENTATION | DM_PAPERSIZE;
                {
                    let s = (*dm).u1.s1_mut();
                    s.dmOrientation = if landscape {
                        DMORIENT_LANDSCAPE
                    } else {
                        DMORIENT_PORTRAIT
                    } as i16;
                    s.dmPaperSize = DMPAPER_A4 as i16;
                }
                ResetDCW(hdc, dm);
                GlobalUnlock(pd.hDevMode);
            }
        }

        let dpi_x = GetDeviceCaps(hdc, LOGPIXELSX as i32) as f32;
        let dpi_y = GetDeviceCaps(hdc, LOGPIXELSY as i32) as f32;
        let off_x = GetDeviceCaps(hdc, PHYSICALOFFSETX as i32) as f32;
        let off_y = GetDeviceCaps(hdc, PHYSICALOFFSETY as i32) as f32;
        let mmx = dpi_x / 25.4;
        let mmy = dpi_y / 25.4;
        let to_dev = |r: RectMm| {
            (
                (r.x * mmx - off_x).round() as i32,
                (r.y * mmy - off_y).round() as i32,
                (r.w * mmx).round() as i32,
                (r.h * mmy).round() as i32,
            )
        };

        let doc_name: Vec<u16> = "Foto A4".encode_utf16().chain(std::iter::once(0)).collect();
        let mut di: DOCINFOW = zeroed();
        di.cbSize = size_of::<DOCINFOW>() as i32;
        di.lpszDocName = doc_name.as_ptr();

        if StartDocW(hdc, &di) <= 0 {
            DeleteDC(hdc);
            return Err("falha ao iniciar o documento de impressão".into());
        }
        StartPage(hdc);
        SetStretchBltMode(hdc, HALFTONE as i32);
        SetBrushOrgEx(hdc, 0, 0, std::ptr::null_mut());

        // Recorta na área de impressão definida pelas bordas
        let (ax, ay, aw, ah) = to_dev(area);
        IntersectClipRect(hdc, ax, ay, ax + aw, ay + ah);

        // Molduras pretas atrás das fotos (primeira passada, para não cobrir
        // nenhuma foto vizinha)
        if frame_mm > 0.0 {
            let brush = GetStockObject(BLACK_BRUSH as i32) as HBRUSH;
            for (_, _, dest) in items {
                let (x, y, w, h) = to_dev(frame_of(*dest, frame_mm));
                let rect = RECT {
                    left: x,
                    top: y,
                    right: x + w,
                    bottom: y + h,
                };
                FillRect(hdc, &rect, brush);
            }
        }

        let mut draw_err = false;
        for (img, crop, dest) in items {
            // Aplica o recorte (uv 0..1) na imagem original
            let iw = img.width();
            let ih = img.height();
            let sx = (crop[0] * iw as f32).round() as u32;
            let sy = (crop[1] * ih as f32).round() as u32;
            let sw = (((crop[2] - crop[0]) * iw as f32).round() as u32)
                .max(1)
                .min(iw.saturating_sub(sx).max(1));
            let sh = (((crop[3] - crop[1]) * ih as f32).round() as u32)
                .max(1)
                .min(ih.saturating_sub(sy).max(1));

            let cropped: RgbaImage =
                if sx == 0 && sy == 0 && sw == iw && sh == ih {
                    (**img).clone()
                } else {
                    image::imageops::crop_imm(img.as_ref(), sx, sy, sw, sh).to_image()
                };

            let (w, h) = (cropped.width() as i32, cropped.height() as i32);
            let mut buf: Vec<u8> = Vec::with_capacity((w as usize) * (h as usize) * 4);
            for px in cropped.pixels() {
                buf.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }

            let mut bmi: BITMAPINFO = zeroed();
            bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w;
            bmi.bmiHeader.biHeight = -h; // top-down
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB;

            let (px, py, pw, ph) = to_dev(*dest);
            let r = StretchDIBits(
                hdc,
                px,
                py,
                pw,
                ph,
                0,
                0,
                w,
                h,
                buf.as_ptr() as *const _,
                &bmi,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
            if r == 0 {
                draw_err = true;
            }
        }

        EndPage(hdc);
        EndDoc(hdc);
        DeleteDC(hdc);
        if !pd.hDevMode.is_null() {
            GlobalFree(pd.hDevMode);
        }
        if !pd.hDevNames.is_null() {
            GlobalFree(pd.hDevNames);
        }

        if draw_err {
            return Err("falha ao desenhar uma ou mais fotos na impressora".into());
        }
        Ok(true)
    }
}

#[cfg(not(windows))]
fn print_photos(
    _: &[(Arc<RgbaImage>, [f32; 4], RectMm)],
    _: RectMm,
    _: bool,
    _: f32,
    _: isize,
) -> Result<bool, String> {
    Err("impressão disponível apenas no Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Roda com: $env:HEIC_TEST = "caminho\foto.heic"; cargo test -- --nocapture
    #[test]
    fn decode_heic_if_available() {
        if let Ok(p) = std::env::var("HEIC_TEST") {
            let img = decode_image(std::path::Path::new(&p)).expect("falha ao decodificar HEIC");
            assert!(img.width() > 0 && img.height() > 0);
            println!("HEIC decodificado: {}x{} px", img.width(), img.height());
        }
    }

    /// Roda com: $env:PDF_TEST = "caminho\arquivo.pdf"; cargo test -- --nocapture
    #[test]
    fn decode_pdf_if_available() {
        if let Ok(p) = std::env::var("PDF_TEST") {
            let path = std::path::Path::new(&p);
            let (img, count, _) = render_pdf_page(path, 0).expect("falha ao renderizar PDF");
            assert!(img.width() > 0 && img.height() > 0 && count > 0);
            println!(
                "PDF: {} página(s); página 1 = {}x{} px",
                count,
                img.width(),
                img.height()
            );
        }
    }
}

fn main() -> eframe::Result {
    // Impede o libheif de varrer diretórios atrás de plugins externos
    // (os codecs necessários já estão embutidos estaticamente)
    std::env::set_var("LIBHEIF_PLUGIN_PATH", "");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 800.0])
            .with_min_inner_size([820.0, 600.0])
            .with_title("Impressão de Foto A4"),
        ..Default::default()
    };
    eframe::run_native(
        "Impressão de Foto A4",
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}
