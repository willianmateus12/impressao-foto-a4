<p align="center">
  <img src="assets/logo.png" width="128" alt="Impressão de Foto A4">
</p>

<h1 align="center">Impressão de Foto A4</h1>

<p align="center">
  Software para Windows que imprime fotos e PDFs em folha A4, com ajuste de bordas, colagem automática, recorte e posicionamento livre com ancoragem (snap) nas extremidades.
</p>

## Download

Baixe o instalador (`ImpressaoFotoA4-Setup-x.y.z.exe`) na página de **[Releases](../../releases/latest)**. Ele instala o programa, cria atalho no Menu Iniciar (e opcionalmente na Área de Trabalho) e registra o desinstalador. Não precisa de nenhum programa extra.

## Formatos aceitos

**JPG, PNG, HEIC/HEIF (fotos de iPhone), AVIF, BMP, WEBP, TIFF, GIF e PDF.**

- HEIC/HEIF é decodificado pelo **libheif/libde265 embutido no executável** — funciona em qualquer PC, sem instalar codec do Windows. O decodificador do Windows (WIC) fica como reserva automática.
- PDF é aberto pelo **motor de PDF nativo do Windows** (`Windows.Data.Pdf`, o mesmo do Edge). Cada página é renderizada a 200 DPI e tratada como uma foto. Se o PDF tiver mais de uma página, aparece um seletor de páginas no painel lateral (◀ ▶ e campo numérico); trocar de página preserva o ajuste já feito.

## Como usar

1. **Adicionar arquivos** — clique em "➕ Adicionar arquivo(s)..." (pode selecionar vários de uma vez) ou arraste imagens/PDFs do Explorer para a janela. Vários arquivos podem coexistir na mesma folha.
   - **Arrastar do Explorer sobre uma foto existente substitui aquela foto**; arrastar sobre uma área vazia da folha adiciona uma nova. Enquanto arrasta, um destaque verde ("Substituir") ou azul ("Adicionar") mostra o que vai acontecer.
2. **Tamanho ao adicionar** — escolha entre:
   - **Tamanho real** (padrão): o PDF entra no tamanho físico da página (um PDF A4 ocupa a folha inteira, um A5 fica em A5); a foto entra pela resolução (DPI) gravada no arquivo, ou 300 DPI se não informada (72/96 DPI, o padrão de tela gravado por celulares e câmeras, não conta). O arquivo é centralizado na área de impressão.
   - **Caber na folha**: preenche a área de impressão, montando colagem automática quando há mais de um arquivo.
3. **Orientação da folha** — muda sozinha conforme o arquivo adicionado: foto ou PDF em paisagem vira a folha para Paisagem; em retrato, para Retrato. Os botões Retrato/Paisagem continuam disponíveis para trocar manualmente.
4. **Ajustar as bordas** — defina cada borda (esquerda, superior, direita, inferior) em milímetros. Os botões `0`, `5` e `10` aplicam o valor nas quatro bordas de uma vez (`0` = folha toda, sem bordas).
5. **Posicionar a foto** — arraste a foto sobre a folha A4 branca. Quando uma extremidade chega perto da borda da área de impressão (ou da folha), ela **ancora automaticamente** e aparece uma linha-guia azul.
6. **Redimensionar** — puxe as 8 alças (cantos e meios dos lados). A opção "Manter proporção (cantos)" trava a proporção original ao puxar pelos cantos; **Shift** ao arrastar um canto faz o mesmo.
7. **Atalhos** — "Preencher área" estica a foto na área toda; "Ajustar proporção" encaixa sem distorcer; "Centralizar" centraliza na área; "Trazer p/ frente" coloca a foto selecionada sobre as outras; "Remover" exclui a selecionada.
8. **Recortar** — selecione uma foto e clique em "✂ Recortar" (ou dê **duplo-clique** nela): arraste as alças para definir a área mantida e arraste dentro da seleção para reposicionar. "✔ Concluir recorte" aplica; "↺ Recorte total" volta à imagem inteira; `Esc` também conclui. Cada foto tem seu próprio recorte.
9. **Várias fotos (colagem automática)** — no modo "Caber na folha", as fotos são montadas em **colagem estilo editor** preenchendo a folha (ex.: 4 fotos = uma grande + 3 ao lado; 2 = lado a lado; etc.), cada uma cobrindo sua célula sem distorcer. O botão "▦ Organizar em colagem" refaz o arranjo a qualquer momento. Ao mover/redimensionar, a foto sai da colagem e passa a ser livre.
   - **Ancoragem inteligente**: ao arrastar, a foto ancora nas bordas da folha, da área de impressão **e das outras fotos**. Com a moldura preta ligada, a referência é a borda preta externa.
10. **Remover** — selecione e pressione `Delete` (ou `Backspace`), use o botão "🗑 Remover", ou clique com o **botão direito** sobre a foto e escolha "🗑 Excluir foto" (o menu também traz Recortar, Trazer para frente e Centralizar).
11. **Moldura preta** — ligada por padrão, desenha uma moldura/separação preta entre as fotos (útil como guia de corte). Uma barra deslizante ajusta a espessura (0–15 mm); desmarque para não imprimir moldura.
12. **Imprimir** — clique em "🖨 Imprimir...", escolha a impressora e confirme. Todas as fotos da folha (e a moldura preta, se ativa) são impressas juntas; o papel é forçado para A4 e a orientação segue a escolhida no aplicativo.

O painel lateral mostra o tamanho da foto selecionada em cm e a resolução efetiva (DPI) para avaliar a qualidade da impressão.

## Observações

- A área fora das bordas é recortada na impressão, exatamente como mostrado na pré-visualização.
- Para impressão realmente sem margem nenhuma, a impressora precisa suportar modo "sem bordas" (borderless); caso contrário o driver corta a margem física mínima.
- A pré-visualização usa uma versão reduzida da foto para desempenho, mas a impressão sempre usa a resolução original.

## Compilar

Pré-requisito (uma única vez): [Rust](https://rustup.rs) e libheif/libde265 estáticos via vcpkg — o caminho do vcpkg está em `.cargo\config.toml` (`VCPKG_ROOT = C:\tmp\vcpkg`):

```
git clone https://github.com/microsoft/vcpkg C:\tmp\vcpkg
C:\tmp\vcpkg\bootstrap-vcpkg.bat -disableMetrics
C:\tmp\vcpkg\vcpkg install "libheif[core]:x64-windows-static-md"
```

Depois:

```
cargo build --release
```

O executável fica em `target\release\foto_a4.exe` (autossuficiente, basta copiar).

### Instalador

Requer o [Inno Setup 6](https://jrsoftware.org/isinfo.php):

```
"C:\Program Files (x86)\Inno Setup 6\ISCC.exe" installer.iss
```

Gera `installer\ImpressaoFotoA4-Setup-x.y.z.exe`.

## Licença

[MIT](LICENSE)
