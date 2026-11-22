; P20: how long the 68000 is held by writes to the VDP data port. At line 60 the HV counter is read, N words are written to the
; data port (VRAM write, increment 2), the HV counter is read again; the word stored is the difference of the two HV words.
; Ten cases: display off then display on, N = 0, 2, 4, 6, 8.
; Result at RAM offset 0: ten words (display off N=0,2,4,6,8, then display on N=0,2,4,6,8).
    .org $200
    lea     $C00000,a0
    lea     $C00004,a1
    lea     $FF0000,a2
    lea     table(pc),a3
    move.w  #$8004,(a1)
    move.w  #$8F02,(a1)
    move.w  #$8230,(a1)
    move.w  #$8B00,(a1)
    move.w  #$8C81,(a1)
    move.l  #$40000000,(a1)
    move.w  #$1234,d2
    moveq   #9,d6
outer:
    move.w  (a3)+,(a1)
    move.w  (a3)+,d5
sync:
    move.w  $C00008,d3
    sub.w   #$3C00,d3
    cmp.w   #$30,d3
    bcc.s   sync
    move.w  $C00008,d1
    move.w  d5,d0
    beq.s   skip
    subq.w  #1,d0
wr: move.w  d2,(a0)
    dbf     d0,wr
skip:
    move.w  $C00008,d4
    sub.w   d1,d4
    move.w  d4,(a2)+
    dbf     d6,outer
forever:
    bra.s   forever
table:
    dc.w    $8104,0,$8104,2,$8104,4,$8104,6,$8104,8
    dc.w    $8144,0,$8144,2,$8144,4,$8144,6,$8144,8
