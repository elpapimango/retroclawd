; sgx: a SuperGrafx probe card. Both video controllers draw a background and four 32x32 sprites; the video priority
; controller's registers come from the table at $FF00 (patched per variant by build.py), so one card shows how the
; two pictures are combined.
;   VDC 1 (ports $0000-$0003): background red (palette 0) in the top half; sprites blue (sprite palette 0)
;   VDC 2 (ports $0010-$0013): background green (palette 1) in the left half; sprites yellow (sprite palette 1),
;   each 16 dots right of and below VDC 1's
; So the quadrants are: top left both backgrounds, top right VDC 1's, bottom left VDC 2's, bottom right none
; (backdrop grey); a blue and a yellow sprite overlap in each quadrant.
    .include "common.inc"
    .org $E000

; a video controller register; the controller is named by its three ports as literals (`$0000,$0002,$0003`): an
; expression like `base+2` would give the value $0002, which assembles as a zero-page store ($2002 on this CPU)
.macro vreg sel,lo_p,hi_p,reg,lo,hi
    lda #reg
    sta sel
    lda #lo
    sta lo_p
    lda #hi
    sta hi_p
.endm

; point the write address at `addr` and select the VRAM data register
.macro vaddr sel,lo_p,hi_p,addr
    vreg sel,lo_p,hi_p,$00,(addr & $FF),(addr >> 8)
    lda #$02
    sta sel
.endm

; write `count` (1-256) words of value `word`
.macro vrun sel,lo_p,hi_p,count,word
    .local again
    ldx #(count & $FF)
again:
    lda #(word & $FF)
    sta lo_p
    lda #(word >> 8)
    sta hi_p
    dex
    bne again
.endm

; a palette entry
.macro colour entry,grb
    put (entry & $FF),$0402
    put (entry >> 8),$0403
    put (grb & $FF),$0404
    put (grb >> 8),$0405
.endm

; the tiles and sprite patterns one controller needs, and its display registers
.macro common sel,lo_p,hi_p
    .local cell
    ; tile $100 at $1000: colour 1 everywhere (plane 0 set); tile $101 at $1010: empty
    vaddr sel,lo_p,hi_p,$1000
    vrun sel,lo_p,hi_p,8,$00FF
    vrun sel,lo_p,hi_p,8,$0000
    vrun sel,lo_p,hi_p,16,$0000
    ; sprite pattern $100 (VRAM $2000): four 16x16 cells of colour 1
    vaddr sel,lo_p,hi_p,$2000
    ldy #4
cell:
    vrun sel,lo_p,hi_p,16,$FFFF
    vrun sel,lo_p,hi_p,48,$0000
    dey
    bne cell
    ; display: 256 x 240, the values most games use
    vreg sel,lo_p,hi_p,$0A,$02,$02
    vreg sel,lo_p,hi_p,$0B,$1F,$04
    vreg sel,lo_p,hi_p,$0C,$02,$0F
    vreg sel,lo_p,hi_p,$0D,$EF,$00
    vreg sel,lo_p,hi_p,$0E,$03,$00
    vreg sel,lo_p,hi_p,$09,$00,$00
    vreg sel,lo_p,hi_p,$07,$00,$00
    vreg sel,lo_p,hi_p,$08,$00,$00
.endm

; the sprite table at $7F00 from 32 bytes at `table` (four sprites), the other 60 sprites cleared, then the DMA set up
.macro sprites sel,lo_p,hi_p,table
    .local copy
    vaddr sel,lo_p,hi_p,$7F00
    ldx #0
copy:
    lda table,x
    sta lo_p
    inx
    lda table,x
    sta hi_p
    inx
    cpx #32
    bne copy
    vrun sel,lo_p,hi_p,240,$0000
    vreg sel,lo_p,hi_p,$0F,$10,$00          ; DCR: the sprite table DMA every frame
    vreg sel,lo_p,hi_p,$13,$00,$7F          ; SATB
    vreg sel,lo_p,hi_p,$05,$C0,$00          ; CR: background and sprites on, no interrupts
.endm

start:
    preamble
    ; the priority controller first: VDC 2 gets no ST instructions
    put $00,$000E
    ; palette
    colour $000,$0092              ; backdrop grey
    colour $001,$0038              ; background palette 0 colour 1: red
    colour $011,$01C0              ; background palette 1 colour 1: green
    colour $100,$0000              ; border black
    colour $101,$0007              ; sprite palette 0 colour 1: blue
    colour $111,$01F8              ; sprite palette 1 colour 1: yellow

    common $0000,$0002,$0003
    common $0010,$0012,$0013

    ; VDC 1 map (32 x 32 at $0000): rows 0-13 tile $100 palette 0, rows 14-31 empty
    vaddr $0000,$0002,$0003,$0000
    vrun $0000,$0002,$0003,256,$0100
    vrun $0000,$0002,$0003,192,$0100
    vrun $0000,$0002,$0003,256,$0101
    vrun $0000,$0002,$0003,256,$0101
    vrun $0000,$0002,$0003,64,$0101
    ; VDC 2 map: columns 0-15 tile $100 palette 1, 16-31 empty, every row
    vaddr $0010,$0012,$0013,$0000
    ldy #32
row:
    vrun $0010,$0012,$0013,16,$1100
    vrun $0010,$0012,$0013,16,$0101
    dey
    bne row

    sprites $0000,$0002,$0003,sat1
    sprites $0010,$0012,$0013,sat2

    ; the priority controller's registers from the table
    lda vpc+0
    sta $0008
    lda vpc+1
    sta $0009
    lda vpc+2
    sta $000A
    lda vpc+3
    sta $000B
    lda vpc+4
    sta $000C
    lda vpc+5
    sta $000D
spin:
    jmp spin

; sprite entries: y + 64, x + 32, pattern ($2000 >> 5), attributes (32 x 32, in front of the background, palette)
.macro spr x,y,pal
    .word y + 64, x + 32, $0100, $1180 | pal
.endm
sat1:
    spr 40,40,0
    spr 168,40,0
    spr 40,150,0
    spr 168,150,0
sat2:
    spr 56,56,1
    spr 184,56,1
    spr 56,166,1
    spr 184,166,1

    .org $FF00
; the priority controller's registers $0008-$000D: priorities (two nibbles each), window 1 width, window 2 width;
; each variant sets them with -D (build.py); without, both controllers are on everywhere in mode 0
    .ifndef VPC8
VPC8 = $33
    .endif
    .ifndef VPC9
VPC9 = $33
    .endif
    .ifndef WIN1
WIN1 = 0
    .endif
    .ifndef WIN2
WIN2 = 0
    .endif
vpc:
    .byte VPC8, VPC9, (WIN1 & $FF), (WIN1 >> 8), (WIN2 & $FF), (WIN2 >> 8)

    .org $FFF6
    .word $EAEA,$EAEA,$EAEA,$EAEA,start
