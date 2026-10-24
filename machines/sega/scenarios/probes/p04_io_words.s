; P04: the I/O registers read as words and at even byte addresses, power-on values, and what a data write does.
; Result at RAM offset 0:
;   $00: words read from $A10000, $A10002, $A10004, $A10006, $A10008, $A1000A, $A1000C, $A1000E (8 words)
;   $10: bytes read from $A10000, $A10002, $A10008 (even addresses)
;   $14: after control = $40 and data = $40 on port 1: data read; after data = $00: data read; control read-back
;   $18: after control = $7F on port 1 and data = $15: data read
;   $1A: tx/rx/serial control: write $A5 to $A1000F, read $A1000F, $A10011, $A10013; write $F8 to $A10013, read it back
    .org $200
    lea     $FF0000,a2
    lea     $A10000,a3
    move.w  $A10000,(a2)+
    move.w  $A10002,(a2)+
    move.w  $A10004,(a2)+
    move.w  $A10006,(a2)+
    move.w  $A10008,(a2)+
    move.w  $A1000A,(a2)+
    move.w  $A1000C,(a2)+
    move.w  $A1000E,(a2)+
    move.b  $A10000,(a2)+
    move.b  $A10002,(a2)+
    move.b  $A10008,(a2)+
    move.b  #0,(a2)+
    move.b  #$40,$A10009
    move.b  #$40,$A10003
    move.b  $A10003,(a2)+
    move.b  #$00,$A10003
    move.b  $A10003,(a2)+
    move.b  $A10009,(a2)+
    move.b  #0,(a2)+
    move.b  #$7F,$A10009
    move.b  #$15,$A10003
    move.b  $A10003,(a2)+
    move.b  #0,(a2)+
    move.b  #$A5,$A1000F
    move.b  $A1000F,(a2)+
    move.b  $A10011,(a2)+
    move.b  $A10013,(a2)+
    move.b  #$F8,$A10013
    move.b  $A10013,(a2)+
forever:
    bra.s   forever
