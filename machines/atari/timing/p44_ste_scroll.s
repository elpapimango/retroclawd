; p44_ste_scroll.s: the STE's pixel scroll (S17b): the bars of p43 in rows of 21 groups (168 bytes, the extra group a scroll with
; prefetch fetches), drawn with the pixel scroll register $FF8265 at 5. Low resolution, base $20000; palette entry n is a grey
; of 4 bits ($nnn with the guns' bit 3 as their least significant bit): 16 bars of 16 pixels, each a colour index; the

    .org $90000
    move.w  #$2700,sr
    clr.b   $fffa07
    clr.b   $fffa09
    clr.b   $fffa13
    clr.b   $fffa15
    lea     $20000,a0
    move.w  #8399,d0
clear:
    clr.l   (a0)+
    dbra    d0,clear
    ; the bars: 200 lines of 20 groups; group g (0 to 15) has colour g, groups 16 to 19 colour 15
    lea     $20000,a0
    move.w  #199,d3
rows:
    moveq   #0,d2
groups:
    move.w  d2,d4
    cmp.w   #15,d4
    ble.s   low
    moveq   #15,d4
low:
    moveq   #3,d5
planes:
    btst    d5,d4
    sne     d6
    ext.w   d6
    move.w  d6,(a0)+
    dbra    d5,planes
    addq.w  #1,d2
    cmp.w   #21,d2
    blt.s   groups
    dbra    d3,rows
    ; the plane words are stored plane 3 first: fix by reversing each group's four words
    lea     $20000,a0
    move.w  #4199,d0
swap:
    move.w  (a0),d1
    move.w  6(a0),(a0)
    move.w  d1,6(a0)
    move.w  2(a0),d1
    move.w  4(a0),2(a0)
    move.w  d1,4(a0)
    addq.l  #8,a0
    dbra    d0,swap
    move.b  #$02,$ff8201
    clr.b   $ff8203
    clr.b   $ff820d
    clr.b   $ff8260
    move.b  #5,$ff8265
    lea     $ff8240,a1
    lea     greys(pc),a0
    moveq   #15,d0
pal:
    move.w  (a0)+,(a1)+
    dbra    d0,pal
done:
    bra.s   done

greys:
    dc.w    $000,$111,$222,$333,$444,$555,$666,$777,$888,$999,$aaa,$bbb,$ccc,$ddd,$eee,$fff
