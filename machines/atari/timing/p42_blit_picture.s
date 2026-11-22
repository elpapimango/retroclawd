; p42_blit_picture.s: a picture made by the Blitter on an STE (S19). Low resolution, base $20000, 16 STE colours.
;  A: halftone fill (HOP 1) of plane 0 over 100 lines;
;  B: a copy (HOP 2) of that region to plane 1 lower down, skewed 5 with FXSR and end masks;
;  C: halftone AND source (HOP 3) XORed (OP 6) into plane 2 over 40 lines, skew 9 with NFSR, end masks.
    .org $90000
    move.w  #$2700,sr
    clr.b   $fffa07
    clr.b   $fffa09
    clr.b   $fffa13
    clr.b   $fffa15
    lea     $20000,a0
    move.w  #7999,d0
clear:
    clr.l   (a0)+
    dbra    d0,clear
    move.b  #$02,$ff8201
    clr.b   $ff8203
    clr.b   $ff820d
    clr.b   $ff8260
    lea     palette(pc),a0
    lea     $ff8240,a1
    moveq   #15,d0
pal:
    move.w  (a0)+,(a1)+
    dbra    d0,pal
    lea     $ff8a00,a1
    lea     halftone(pc),a0
    moveq   #15,d0
ht:
    move.w  (a0)+,(a1)+
    dbra    d0,ht
    lea     $ff8a00,a1
    ; A
    move.w  #8,$20(a1)
    move.w  #8,$22(a1)
    move.w  #8,$2e(a1)
    move.w  #8,$30(a1)
    move.w  #$ffff,$28(a1)
    move.w  #$ffff,$2a(a1)
    move.w  #$ffff,$2c(a1)
    move.l  #$20000+160*20,$32(a1)
    move.w  #20,$36(a1)
    move.w  #100,$38(a1)
    move.b  #1,$3a(a1)
    move.b  #3,$3b(a1)
    clr.b   $3d(a1)
    move.b  #$c0,$3c(a1)
wa:
    btst    #7,$3c(a1)
    bne.s   wa
    ; B
    move.l  #$20000+160*20,$24(a1)
    move.l  #$20000+160*130+2,$32(a1)
    move.w  #20,$36(a1)
    move.w  #60,$38(a1)
    move.w  #$7fff,$28(a1)
    move.w  #$ffff,$2a(a1)
    move.w  #$fffe,$2c(a1)
    move.b  #2,$3a(a1)
    move.b  #3,$3b(a1)
    move.b  #$85,$3d(a1)
    move.b  #$c0,$3c(a1)
wb:
    btst    #7,$3c(a1)
    bne.s   wb
    ; C
    move.l  #$20000+160*20+16,$24(a1)
    move.l  #$20000+160*30+4,$32(a1)
    move.w  #10,$36(a1)
    move.w  #40,$38(a1)
    move.w  #$0fff,$28(a1)
    move.w  #$aaaa,$2a(a1)
    move.w  #$fff0,$2c(a1)
    move.b  #3,$3a(a1)
    move.b  #6,$3b(a1)
    move.b  #$49,$3d(a1)
    move.b  #$c0,$3c(a1)
wc:
    btst    #7,$3c(a1)
    bne.s   wc
done:
    bra.s   done

halftone:
    dc.w    $8001,$4002,$2004,$1008,$0810,$0420,$0240,$0180,$f00f,$e11e,$d22d,$c33c,$b44b,$a55a,$9669,$8778
palette:
    dc.w    $000,$fff,$f00,$0f0,$00f,$ff0,$0ff,$f0f,$888,$777,$a50,$5a0,$05a,$a05,$50a,$aaa
