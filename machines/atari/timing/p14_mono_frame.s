; p14_mono_frame.s: how long a frame is on the monochrome monitor.
; Frames 2 to 11 run with the HBL masked (SR $2300) and a counted loop: the loop count a frame, as a long, in order.
; Frames 13 to 23 run with the HBL taken (SR $2100) and count the HBLs of the frame: the lines, as a word, in order.
; Result at $C000: header 'P14', then 10 longs, then 11 words.
    .org $90000
    move.w  #$2700,sr
    clr.b   $fffa07             ; IERA, IERB, IMRA, IMRB: no MFP interrupt reaches the CPU
    clr.b   $fffa09
    clr.b   $fffa13
    clr.b   $fffa15
    lea     hbl(pc),a0
    move.l  a0,$68
    lea     vbl(pc),a0
    move.l  a0,$70
    lea     $c000,a2
    move.l  #$50313400,(a2)+
    moveq   #0,d5               ; loop count
    moveq   #0,d6               ; frames
    moveq   #0,d7               ; HBLs
    move.w  #$2300,sr
loop:
    addq.l  #1,d5
    bra.s   loop

vbl:
    addq.w  #1,d6
    cmp.w   #12,d6
    bge.s   hblphase
    move.w  #$2300,(sp)
    cmp.w   #2,d6
    blt.s   clear
    move.l  d5,(a2)+
clear:
    moveq   #0,d5
    moveq   #0,d7
    rte
hblphase:
    move.w  #$2100,(sp)
    cmp.w   #13,d6
    blt.s   clear
    move.w  d7,(a2)+
    cmp.w   #23,d6
    blt.s   clear
    ori.w   #$0700,(sp)
    bra.s   clear

hbl:
    addq.w  #1,d7
    rte
